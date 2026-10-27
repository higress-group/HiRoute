use super::*;
use hiroute_domain::{ApiEquivalentPriceV1, ObservationMetricCoverageV2, PriceValuationKindV1};

fn reference_price(binding: &str, model: &str, rate: u64) -> ApiEquivalentPriceV1 {
    let mut quote = price(100, UsageFrameKindV1::Cumulative).quote.unwrap();
    quote.exact_target.valuation_kind = PriceValuationKindV1::ApiEquivalent;
    quote.valuation_kind = PriceValuationKindV1::ApiEquivalent;
    quote.exact_target.model_identity = PriceModelIdentityV1::CatalogModel(model.into());
    quote.rates = TokenRatesV1 {
        input_uncached: TokenRateV1::known(rate),
        output: TokenRateV1::known(rate),
        cache_read: TokenRateV1::known(rate),
        cache_write: TokenRateV1::known(rate),
    };
    quote.quote_digest = quote.computed_digest().unwrap();
    ApiEquivalentPriceV1 {
        binding_id: binding.into(),
        execution_model_configuration_id: None,
        quote,
    }
}
fn reader(fixture: &Fixture) -> ObservationReaderContext {
    ObservationReaderContext::local_user(
        fixture.workspace.clone(),
        "local-estimate".into(),
        1,
        10_000,
        false,
        false,
    )
    .unwrap()
}
fn query(fixture: &Fixture) -> ObservationValueQueryV2 {
    ObservationValueQueryV2 {
        from_ms: 50,
        to_ms: 1000,
        session_id: Some(fixture.session.to_string()),
        plan_id: Some("plan/codex-daily".into()),
        currency: Some("USD".into()),
    }
}

#[test]
fn routed_total_remains_available_when_reference_requires_unreported_cache_write() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let fixture = Fixture::new("separate-routed-coverage");
    for mut envelope in facts(&fixture, UsageFrameKindV1::Cumulative) {
        if let ExecutionFactV1::UsageAndCache {
            cache_write_tokens,
            cache_write_provenance,
            ..
        } = &mut envelope.fact
        {
            *cache_write_tokens = None;
            *cache_write_provenance = UsageProvenanceV1::Unknown;
        }
        store.ingest_fact(&envelope, &[]).unwrap();
    }
    store.settle_pending_valuations(16).unwrap();
    let frozen = serde_json::to_value(latest(&store, &fixture)).unwrap();
    let actual = reference_price("binding-a", "model-a", 1_000_000);
    let mut reference = reference_price("binding-ref", "model-reference", 4_000_000);
    reference.quote.rates.cache_write = TokenRateV1::known(5_000_000);
    reference.quote.quote_digest = reference.quote.computed_digest().unwrap();

    let read = || {
        store
            .observed_api_equivalent_value(
                &reader(&fixture),
                &query(&fixture),
                &reference,
                std::slice::from_ref(&actual),
                1000,
            )
            .unwrap()
    };
    let indexed = read();
    assert_eq!(indexed.routed_estimate_micros, Some(125));
    assert_eq!(indexed.reference_estimate_micros, None);
    assert_eq!(indexed.estimated_savings_micros, None);
    assert_eq!(indexed.compared_request_count, 0);
    assert_eq!(indexed.coverage, ObservationMetricCoverageV2::Unknown);
    assert_eq!(
        indexed.same_usage_bounds,
        Some(ApiEquivalentBoundsV1 {
            reference_min_micros: 500,
            reference_max_micros: 560,
            difference_min_micros: -435,
            difference_max_micros: -375,
        })
    );
    assert_eq!(
        serde_json::to_value(latest(&store, &fixture)).unwrap(),
        frozen
    );

    // The documented retained-reader fallback must produce the same result.
    store
        .connection
        .lock()
        .execute(
            "UPDATE observation_analysis_meta_v1 SET value='future' WHERE key='format'",
            [],
        )
        .unwrap();
    let fallback = read();
    assert_eq!(
        fallback.routed_estimate_micros,
        indexed.routed_estimate_micros
    );
    assert_eq!(fallback.reference_estimate_micros, None);
    assert_eq!(fallback.estimated_savings_micros, None);
    assert_eq!(fallback.compared_request_count, 0);
    assert_eq!(fallback.same_usage_bounds, indexed.same_usage_bounds);
}
#[test]
fn explicit_current_price_comparison_keeps_frozen_values_and_missing_evidence_honest() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let fixture = Fixture::new("api-estimate");
    for fact in facts(&fixture, UsageFrameKindV1::Cumulative) {
        store.ingest_fact(&fact, &[]).unwrap();
    }
    store.settle_pending_valuations(16).unwrap();
    let frozen = serde_json::to_value(latest(&store, &fixture)).unwrap();
    let cheap = reference_price("binding-a", "model-a", 1_000_000);
    let expensive = reference_price("binding-ref", "model-reference", 4_000_000);
    let report = store
        .observed_api_equivalent_value(
            &reader(&fixture),
            &query(&fixture),
            &expensive,
            std::slice::from_ref(&cheap),
            1000,
        )
        .unwrap();
    assert_eq!(
        report.coverage,
        ObservationMetricCoverageV2::Complete,
        "{report:?}"
    );
    assert_eq!(report.request_count, 1);
    assert_eq!(report.compared_request_count, 1);
    assert_eq!(
        report.reference_estimate_micros.unwrap(),
        report.routed_estimate_micros.unwrap() * 4
    );
    assert_eq!(
        report.estimated_savings_micros.unwrap(),
        report.routed_estimate_micros.unwrap() as i64 * 3
    );
    assert_eq!(report.actual_cash_micros, None);
    let negative = store
        .observed_api_equivalent_value(
            &reader(&fixture),
            &query(&fixture),
            &cheap,
            &[reference_price("binding-a", "model-a", 8_000_000)],
            1000,
        )
        .unwrap();
    assert!(negative.estimated_savings_micros.unwrap() < 0);
    assert_eq!(
        serde_json::to_value(latest(&store, &fixture)).unwrap(),
        frozen
    );
    let mut missing = cheap.clone();
    missing.quote.rates.cache_read =
        TokenRateV1::unknown(PriceUnknownReasonV1::CacheRateNotCollected);
    missing.quote.quote_digest = missing.quote.computed_digest().unwrap();
    let unknown = store
        .observed_api_equivalent_value(
            &reader(&fixture),
            &query(&fixture),
            &expensive,
            &[missing],
            1000,
        )
        .unwrap();
    assert_eq!(unknown.compared_request_count, 0);
    assert_eq!(unknown.estimated_savings_micros, None);
    assert_eq!(unknown.routed_estimate_micros, None);
    let mut foreign = cheap;
    foreign.quote.exact_target.source_identity_digest =
        CanonicalDigest::of_bytes(b"another supplier");
    foreign.quote.quote_digest = foreign.quote.computed_digest().unwrap();
    let unknown = store
        .observed_api_equivalent_value(
            &reader(&fixture),
            &query(&fixture),
            &expensive,
            &[foreign],
            1000,
        )
        .unwrap();
    assert_eq!(unknown.estimated_savings_micros, None);
    assert_eq!(unknown.missing_request_count, 1);
    let mut expired = query(&fixture);
    expired.from_ms = 0;
    let long_reader = ObservationReaderContext::local_user(
        fixture.workspace.clone(),
        "local-estimate".into(),
        1,
        crate::managed_text::RETENTION_MS + 10_000,
        false,
        false,
    )
    .unwrap();
    let unknown = store
        .observed_api_equivalent_value(
            &long_reader,
            &expired,
            &expensive,
            &[],
            crate::managed_text::RETENTION_MS + 500,
        )
        .unwrap();
    assert!(unknown.retention_boundary_partial);
    assert_eq!(unknown.estimated_savings_micros, None);
}

#[test]
fn explicit_comparison_includes_failed_retry_usage_without_rewriting_a_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let fixture = Fixture::new("api-retry");
    for fact in facts(&fixture, UsageFrameKindV1::Cumulative) {
        store.ingest_fact(&fact, &[]).unwrap();
    }
    // The input owner has already admitted two observed Attempts for the same request.
    // Keep the accepted ordinal unchanged and add a failed attempt's retained normalized input.
    store.connection.lock().execute("INSERT INTO valuation_attempt_inputs_v2(workspace_id,request_id,ordinal,pricing_json,usage_json) SELECT workspace_id,request_id,2,pricing_json,usage_json FROM valuation_attempt_inputs_v2 WHERE workspace_id=?1 AND request_id=?2 AND ordinal=1",rusqlite::params![fixture.workspace.as_str(),fixture.request.as_str()]).unwrap();
    let cheap = reference_price("binding-a", "model-a", 1_000_000);
    let report = store
        .observed_api_equivalent_value(
            &reader(&fixture),
            &query(&fixture),
            &cheap,
            std::slice::from_ref(&cheap),
            1000,
        )
        .unwrap();
    assert_eq!(
        report.coverage,
        ObservationMetricCoverageV2::Complete,
        "{report:?}"
    );
    assert_eq!(
        report.routed_estimate_micros.unwrap(),
        2 * report.reference_estimate_micros.unwrap()
    );
    assert!(report.estimated_savings_micros.unwrap() < 0);
}

#[test]
fn missing_cache_creation_split_is_irrelevant_only_under_explicit_equal_rates() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let fixture = Fixture::new("api-cache-creation");
    let mut events = facts(&fixture, UsageFrameKindV1::Cumulative);
    for event in &mut events {
        if let ExecutionFactV1::UsageAndCache {
            cache_write_tokens,
            cache_write_provenance,
            ..
        } = &mut event.fact
        {
            *cache_write_tokens = None;
            *cache_write_provenance = UsageProvenanceV1::Unknown;
        }
        store.ingest_fact(event, &[]).unwrap();
    }
    let source = reference_price("binding-a", "model-a", 1_000_000);
    let reference = reference_price("reference", "primary", 4_000_000);
    let report = store
        .observed_api_equivalent_value(
            &reader(&fixture),
            &query(&fixture),
            &reference,
            std::slice::from_ref(&source),
            1000,
        )
        .unwrap();
    assert_eq!(
        report.coverage,
        ObservationMetricCoverageV2::Complete,
        "{report:?}"
    );
    assert!(report.estimated_savings_micros.unwrap() > 0);
    let mut unequal = source;
    unequal.quote.rates.cache_write = TokenRateV1::known(3_000_000);
    unequal.quote.quote_digest = unequal.quote.computed_digest().unwrap();
    let report = store
        .observed_api_equivalent_value(
            &reader(&fixture),
            &query(&fixture),
            &reference,
            &[unequal],
            1000,
        )
        .unwrap();
    assert_eq!(report.estimated_savings_micros, None);
    let raw: String = store
        .connection
        .lock()
        .query_row(
            "SELECT usage_json FROM valuation_attempt_inputs_v2 WHERE request_id=?1 LIMIT 1",
            [fixture.request.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(serde_json::from_str::<serde_json::Value>(&raw).unwrap()["write"].is_null());
}

#[test]
fn local_price_namespace_uses_the_same_exact_recorded_model_configuration() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let fixture = Fixture::new("api-local-model");
    for fact in facts(&fixture, UsageFrameKindV1::Cumulative) {
        store.ingest_fact(&fact, &[]).unwrap();
    }
    let mut local = reference_price("local-binding", "model-a", 1_000_000);
    local.quote.exact_target.model_identity = PriceModelIdentityV1::LocalModel("model-a".into());
    local.quote.quote_digest = local.quote.computed_digest().unwrap();
    let report = store
        .observed_api_equivalent_value(
            &reader(&fixture),
            &query(&fixture),
            &local,
            std::slice::from_ref(&local),
            1000,
        )
        .unwrap();
    assert_eq!(report.coverage, ObservationMetricCoverageV2::Complete);
    assert_eq!(report.estimated_savings_micros, Some(0));
}

#[test]
fn source_local_execution_mapping_keeps_the_canonical_price_and_exact_supplier() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let fixture = Fixture::new("api-execution-model");
    for fact in facts(&fixture, UsageFrameKindV1::Cumulative) {
        store.ingest_fact(&fact, &[]).unwrap();
    }
    let mut price = reference_price("binding-a", "canonical-model", 1_000_000);
    price.execution_model_configuration_id = Some("model-a".into());
    let report = store
        .observed_api_equivalent_value(
            &reader(&fixture),
            &query(&fixture),
            &price,
            std::slice::from_ref(&price),
            1000,
        )
        .unwrap();
    assert_eq!(report.coverage, ObservationMetricCoverageV2::Complete);
    assert_eq!(report.estimated_savings_micros, Some(0));
    assert_eq!(
        report.prices[0].quote.exact_target.model_identity,
        PriceModelIdentityV1::CatalogModel("canonical-model".into())
    );

    let mut wrong = price.clone();
    wrong.execution_model_configuration_id = Some("another-execution-model".into());
    let report = store
        .observed_api_equivalent_value(&reader(&fixture), &query(&fixture), &price, &[wrong], 1000)
        .unwrap();
    assert_eq!(report.compared_request_count, 0);
    let mut foreign = price.clone();
    foreign.quote.exact_target.source_identity_digest =
        CanonicalDigest::of_bytes(b"foreign-source");
    foreign.quote.quote_digest = foreign.quote.computed_digest().unwrap();
    let report = store
        .observed_api_equivalent_value(
            &reader(&fixture),
            &query(&fixture),
            &price,
            &[foreign],
            1000,
        )
        .unwrap();
    assert_eq!(report.compared_request_count, 0);
}
