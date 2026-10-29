use super::*;
use hiroute_domain::*;

fn quote() -> FrozenPriceQuoteV1 {
    let target = PriceTargetV1 {
        workspace_id: WorkspaceId::default(),
        source_id: "source-a".into(),
        source_identity_digest: CanonicalDigest::of_bytes(b"source-a"),
        model_identity: PriceModelIdentityV1::CatalogModel("model-a".into()),
        currency: "USD".into(),
        valuation_kind: PriceValuationKindV1::UsageEstimate,
    };
    let mut quote = FrozenPriceQuoteV1 {
        generation_ref: None,
        actual_source_ref: target.source_id.clone(),
        exact_target: target,
        actual_offer_ref: None,
        reference_model_offer_ref: None,
        valuation_kind: PriceValuationKindV1::UsageEstimate,
        currency: "USD".into(),
        unit: PriceUnitV1::MicrosPerMillionTokens,
        rates: TokenRatesV1 {
            input_uncached: TokenRateV1::known(1_000_000),
            output: TokenRateV1::known(2_000_000),
            cache_read: TokenRateV1::known(100_000),
            cache_write: TokenRateV1::known(1_500_000),
        },
        origin: PriceOriginV1::Manual,
        applied_override_refs: vec![],
        selected_rule_refs: vec![],
        attempt_execution_at: 1,
        applied_schedule: None,
        billing_context: PriceBillingContextV1::StandardTokens,
        unknown_reasons: vec![],
        quote_digest: CanonicalDigest::of_bytes(b"pending"),
    };
    quote.quote_digest = quote.computed_digest().unwrap();
    quote
}

#[test]
fn inclusive_input_is_partitioned_without_double_billing_cache_or_reasoning() {
    let usage = UsageBucketsV2::from_inclusive_input(Some(1000), Some(100), Some(200), Some(100));
    assert_eq!(usage.input_uncached, Some(700));
    let amount = value_attempt(&quote(), &usage).unwrap();
    assert_eq!(amount.known_sum_micros, Some(1070));
    assert_eq!(amount.coverage, MetricCoverage::Complete);
    // Reasoning already included in the output does not create a fifth bucket.
    assert_eq!(amount.components.len(), 4);
}

#[test]
fn unknown_cache_semantics_keep_known_output_and_explicit_partial_coverage() {
    let usage = UsageBucketsV2::from_inclusive_input(Some(1000), Some(100), None, None);
    assert_eq!(usage.input_uncached, None);
    let amount = value_attempt(&quote(), &usage).unwrap();
    assert_eq!(amount.known_sum_micros, Some(200));
    assert_eq!(amount.coverage, MetricCoverage::Partial);
    assert_eq!(
        amount
            .components
            .iter()
            .filter(|part| part.missing.is_some())
            .count(),
        3
    );
}

#[test]
fn frozen_equal_rates_value_an_unknown_write_without_inventing_tokens() {
    let mut price = quote();
    price.rates.cache_write = price.rates.input_uncached.clone();
    price.quote_digest = price.computed_digest().unwrap();
    let mut pricing = ExecutionPricingEvidenceV1 {
        schema_version: EXECUTION_PRICING_SCHEMA_V1.into(),
        request_generation: None,
        captured_at_ms: 1_000,
        attempt_execution_at_ms: 1_000,
        quote: Some(price),
        reference_quote: None,
        unknown_reason: None,
        usage_semantics: UsageSemanticsV1 {
            frame_kind: UsageFrameKindV1::Cumulative,
            input: InputUsageMeaningV1::IncludesExclusiveCache,
            output: OutputUsageMeaningV1::IncludesReasoning,
            cache_buckets_exclusive: true,
        },
    };
    let raw = RawUsage {
        input: Some(100),
        output: Some(25),
        read: Some(40),
        write: None,
        reasoning: None,
    };
    let original = serde_json::to_value(&raw).unwrap();
    let valued = value_recorded_attempt(&pricing, &raw).unwrap();
    assert_eq!(valued.known_sum_micros, Some(114));
    assert_eq!(valued.coverage, MetricCoverage::Complete);
    assert_eq!(valued.algorithm, EQUAL_RATE_VALUATION_ALGORITHM);
    assert_eq!(valued.components[0].tokens, None);
    assert_eq!(valued.components[3].tokens, None);
    assert_eq!(
        valued.components[3].missing,
        Some(ValuationMissingReason::UsageUnknown)
    );
    assert_eq!(serde_json::to_value(&raw).unwrap(), original);

    // Neither a different nor an unknown frozen write price is inferable.
    for write in [
        TokenRateV1::known(1_500_000),
        TokenRateV1::unknown(PriceUnknownReasonV1::CacheRateNotCollected),
    ] {
        let quote = pricing.quote.as_mut().unwrap();
        quote.rates.cache_write = write;
        quote.quote_digest = quote.computed_digest().unwrap();
        let valued = value_recorded_attempt(&pricing, &raw).unwrap();
        assert_eq!(valued.known_sum_micros, Some(54));
        assert_eq!(valued.coverage, MetricCoverage::Partial);
        assert_eq!(valued.algorithm, VALUATION_ALGORITHM);
    }
    let quote = pricing.quote.as_mut().unwrap();
    quote.rates.cache_write = quote.rates.input_uncached.clone();
    quote.quote_digest = quote.computed_digest().unwrap();
    pricing.usage_semantics.cache_buckets_exclusive = false;
    assert_ne!(
        value_recorded_attempt(&pricing, &raw).unwrap().coverage,
        MetricCoverage::Complete
    );

    pricing.usage_semantics.cache_buckets_exclusive = true;
    let quote = pricing.quote.as_mut().unwrap();
    quote.rates.input_uncached = TokenRateV1::known(u64::MAX);
    quote.rates.cache_write = quote.rates.input_uncached.clone();
    quote.quote_digest = quote.computed_digest().unwrap();
    let overflow = RawUsage {
        input: Some(u64::MAX),
        output: Some(0),
        read: Some(0),
        write: None,
        reasoning: None,
    };
    let valued = value_recorded_attempt(&pricing, &overflow).unwrap();
    assert_eq!(valued.known_sum_micros, None);
    assert_eq!(valued.coverage, MetricCoverage::Unknown);
}

#[test]
fn unknown_write_bounds_require_known_exclusive_usage_and_do_not_rewrite_it() {
    let semantics = UsageSemanticsV1 {
        frame_kind: UsageFrameKindV1::Cumulative,
        input: InputUsageMeaningV1::IncludesExclusiveCache,
        output: OutputUsageMeaningV1::IncludesReasoning,
        cache_buckets_exclusive: true,
    };
    let raw = RawUsage {
        input: Some(100),
        output: Some(25),
        read: Some(40),
        write: None,
        reasoning: None,
    };
    let original = serde_json::to_value(&raw).unwrap();
    assert_eq!(
        reference_bounds_semantics(semantics, &raw, &quote()),
        Some((114, 144))
    );
    assert_eq!(serde_json::to_value(&raw).unwrap(), original);

    let mut cheaper_write = quote();
    cheaper_write.rates.cache_write = TokenRateV1::known(500_000);
    assert_eq!(
        reference_bounds_semantics(semantics, &raw, &cheaper_write),
        Some((84, 114))
    );
    let mut missing = raw.clone();
    missing.read = None;
    assert_eq!(
        reference_bounds_semantics(semantics, &missing, &quote()),
        None
    );
    missing.read = Some(101);
    assert_eq!(
        reference_bounds_semantics(semantics, &missing, &quote()),
        None
    );
    let mut ambiguous = semantics;
    ambiguous.cache_buckets_exclusive = false;
    assert_eq!(reference_bounds_semantics(ambiguous, &raw, &quote()), None);
    let mut unpriced = quote();
    unpriced.rates.cache_write = TokenRateV1::unknown(PriceUnknownReasonV1::CacheRateNotCollected);
    assert_eq!(reference_bounds_semantics(semantics, &raw, &unpriced), None);
}

#[test]
fn rounding_happens_once_after_all_attempt_components() {
    let mut price = quote();
    price.rates = TokenRatesV1 {
        input_uncached: TokenRateV1::known(400_000),
        output: TokenRateV1::known(400_000),
        cache_read: TokenRateV1::known(0),
        cache_write: TokenRateV1::known(0),
    };
    price.quote_digest = price.computed_digest().unwrap();
    let amount = value_attempt(
        &price,
        &UsageBucketsV2 {
            input_uncached: Some(1),
            output: Some(1),
            cache_read: Some(0),
            cache_write: Some(0),
        },
    )
    .unwrap();
    assert_eq!(amount.known_sum_micros, Some(1));
}

#[test]
fn absent_price_is_not_free_and_overflow_is_explicit() {
    let mut price = quote();
    price.rates = TokenRatesV1::unknown(PriceUnknownReasonV1::SnapshotUnavailable);
    price.quote_digest = price.computed_digest().unwrap();
    let amount = value_attempt(
        &price,
        &UsageBucketsV2 {
            input_uncached: Some(0),
            output: Some(0),
            cache_read: Some(0),
            cache_write: Some(0),
        },
    )
    .unwrap();
    assert_eq!(amount.coverage, MetricCoverage::Unknown);
    assert_eq!(amount.known_sum_micros, None);
    price.rates = TokenRatesV1 {
        input_uncached: TokenRateV1::known(u64::MAX),
        output: TokenRateV1::known(u64::MAX),
        cache_read: TokenRateV1::known(u64::MAX),
        cache_write: TokenRateV1::known(u64::MAX),
    };
    price.quote_digest = price.computed_digest().unwrap();
    let amount = value_attempt(
        &price,
        &UsageBucketsV2 {
            input_uncached: Some(u64::MAX),
            output: Some(u64::MAX),
            cache_read: Some(u64::MAX),
            cache_write: Some(u64::MAX),
        },
    )
    .unwrap();
    assert_eq!(amount.known_sum_micros, None);
    assert_eq!(amount.coverage, MetricCoverage::Unknown);
}

#[test]
fn altered_quote_and_inconsistent_usage_do_not_produce_complete_cost() {
    let mut price = quote();
    price.currency = "EUR".into();
    assert_eq!(
        value_attempt(&price, &UsageBucketsV2::default()),
        Err(ValuationError::InvalidEvidence)
    );
    let usage = UsageBucketsV2::from_inclusive_input(Some(10), Some(5), Some(9), Some(9));
    assert_eq!(usage.input_uncached, None);
    assert_eq!(usage.cache_read, None);
    assert_eq!(
        value_attempt(&quote(), &usage).unwrap().coverage,
        MetricCoverage::Partial
    );
}

#[test]
fn reference_comparison_includes_retries_preserves_negative_values_and_rejects_mixed_generation() {
    let mut reference = quote();
    reference.generation_ref = Some(hiroute_domain::PriceGenerationRefV1 {
        id: "g-one".into(),
        digest: hiroute_domain::CanonicalDigest::of_bytes(b"g-one"),
        configuration_revision: 1,
        catalog_refs: vec![],
    });
    reference.quote_digest = reference.computed_digest().unwrap();
    let usage = UsageBucketsV2 {
        input_uncached: Some(100),
        output: Some(20),
        cache_read: Some(0),
        cache_write: Some(0),
    };
    let attempt = value_attempt(&reference, &usage).unwrap();
    let cost = attempt.known_sum_micros.unwrap();
    let comparison = compare_reference(
        &reference,
        &usage,
        &[attempt.clone(), attempt.clone()],
        true,
    )
    .unwrap();
    assert_eq!(comparison.baseline_micros, Some(cost));
    assert_eq!(comparison.savings_micros, Some(-(cost as i64)));
    assert_eq!(comparison.actual_cash_micros, None);
    assert!(comparison.disclosure.contains("quality"));
    assert_eq!(
        compare_reference(&reference, &usage, std::slice::from_ref(&attempt), false)
            .unwrap()
            .savings_micros,
        None
    );
    let mut other = reference.clone();
    other.generation_ref.as_mut().unwrap().id = "g-two".into();
    other.quote_digest = other.computed_digest().unwrap();
    assert_eq!(
        compare_reference(&other, &usage, &[attempt], true)
            .unwrap()
            .savings_micros,
        None
    );
}

#[test]
fn known_zero_bucket_needs_no_rate_but_unknown_usage_stays_unknown() {
    let mut price = quote();
    price.rates.cache_write = TokenRateV1::unknown(PriceUnknownReasonV1::CacheRateNotCollected);
    price.quote_digest = price.computed_digest().unwrap();
    let usage = UsageBucketsV2 {
        input_uncached: Some(100),
        output: Some(10),
        cache_read: Some(0),
        cache_write: Some(0),
    };
    assert_eq!(
        value_attempt(&price, &usage).unwrap().coverage,
        MetricCoverage::Complete
    );
    let unknown = UsageBucketsV2 {
        cache_write: None,
        ..usage
    };
    assert_eq!(
        value_attempt(&price, &unknown).unwrap().coverage,
        MetricCoverage::Partial
    );
}
