use super::*;
use hiroute_application::prices::{CatalogPriceRuleV2, PriceIndexEntryV1, PriceSnapshot};
use hiroute_domain::*;
use hiroute_gateway::server::request_plan::RequestPriceBindingV1;

fn identity(seed: &str) -> CanonicalDigest {
    CanonicalDigest::of_bytes(seed.as_bytes())
}

fn entry(source: &str, model: &str, offer: &str, rate: u64) -> PriceIndexEntryV1 {
    PriceIndexEntryV1 {
        execution_binding_id: None,
        target: PriceTargetV1 {
            workspace_id: WorkspaceId::default(),
            source_id: source.into(),
            source_identity_digest: identity(source),
            model_identity: PriceModelIdentityV1::CatalogModel(model.into()),
            currency: "USD".into(),
            valuation_kind: PriceValuationKindV1::UsageEstimate,
        },
        actual_offer_ref: Some(offer.into()),
        reference_model_offer_ref: None,
        legacy_overrides: vec![],
        source_override: None,
        catalog_rules: vec![CatalogPriceRuleV2 {
            rule_ref: PriceFactRefV1 {
                id: format!("rule-{source}-{model}"),
                revision: 1,
                digest: identity(&format!("rule-{source}-{model}")),
            },
            rates: TokenRatesV1::from_legacy(rate, rate),
            schedule: None,
        }],
    }
}

fn binding(
    stable: &str,
    source: &str,
    model: &str,
    profile: &str,
    offer: &str,
) -> RequestPriceBindingV1 {
    RequestPriceBindingV1 {
        stable_binding_id: stable.into(),
        model_configuration_id: model.into(),
        profile_digest: profile.into(),
        source_id: source.into(),
        source_identity_digest: identity(source),
        actual_offer_ref: offer.into(),
        usage_semantics: UsageSemanticsV1 {
            frame_kind: UsageFrameKindV1::Cumulative,
            input: InputUsageMeaningV1::IncludesExclusiveCache,
            output: OutputUsageMeaningV1::IncludesReasoning,
            cache_buckets_exclusive: true,
        },
        billing_context: PriceBillingContextV1::StandardTokens,
    }
}

fn source(slot: Arc<PriceSnapshotSlot>) -> GatewayRequestPriceSource {
    GatewayRequestPriceSource::new(slot)
}

#[test]
fn request_capture_retains_generation_and_converts_actual_attempt_ms_to_seconds() {
    let slot = Arc::new(PriceSnapshotSlot::default());
    slot.install(
        PriceSnapshot::build(
            1,
            vec![],
            vec![entry("source-a", "model-a", "offer-a", 100)],
        )
        .unwrap(),
        None,
    )
    .unwrap();
    let source = source(slot.clone());
    let bindings = [binding(
        "binding", "source-a", "model-a", "profile", "offer-a",
    )];
    let request = source.capture_request(WorkspaceId::DEFAULT, &bindings, 100_999);
    let first = request.freeze_attempt("binding", "model-a", "profile", 101_999);
    assert_eq!(first.quote.as_ref().unwrap().attempt_execution_at, 101);
    slot.install(
        PriceSnapshot::build(
            2,
            vec![],
            vec![entry("source-a", "model-a", "offer-a", 200)],
        )
        .unwrap(),
        first.request_generation.as_ref(),
    )
    .unwrap();
    let fallback = request.freeze_attempt("binding", "model-a", "profile", 102_999);
    assert_eq!(fallback.request_generation, first.request_generation);
    assert_eq!(
        fallback.quote.as_ref().unwrap().rates.input_uncached,
        TokenRateV1::known(100)
    );
    let next = source
        .capture_request(WorkspaceId::DEFAULT, &bindings, 103_999)
        .freeze_attempt("binding", "model-a", "profile", 104_999);
    assert_ne!(next.request_generation, first.request_generation);
    assert_eq!(
        next.quote.as_ref().unwrap().rates.input_uncached,
        TokenRateV1::known(200)
    );
    first.validate().unwrap();
    fallback.validate().unwrap();
    next.validate().unwrap();
}

#[test]
fn unavailable_capture_never_adopts_later_price_and_unverified_profile_is_unknown() {
    let slot = Arc::new(PriceSnapshotSlot::default());
    let source = source(slot.clone());
    let bindings = [binding(
        "binding", "source-a", "model-a", "profile", "offer-a",
    )];
    let request = source.capture_request(WorkspaceId::DEFAULT, &bindings, 1000);
    slot.install(
        PriceSnapshot::build(
            1,
            vec![],
            vec![entry("source-a", "model-a", "offer-a", 100)],
        )
        .unwrap(),
        None,
    )
    .unwrap();
    let quote = request.freeze_attempt("binding", "model-a", "profile", 2000);
    assert!(quote.request_generation.is_none());
    assert_eq!(
        quote.unknown_reason,
        Some(PriceUnknownReasonV1::SnapshotUnavailable)
    );
    let wrong = source
        .capture_request(WorkspaceId::DEFAULT, &bindings, 1000)
        .freeze_attempt("binding", "model-a", "another-profile", 2000);
    assert!(wrong.quote.is_none());
    assert_eq!(
        wrong.unknown_reason,
        Some(PriceUnknownReasonV1::UnverifiedOfferMapping)
    );
}

#[test]
fn request_version_selects_exact_source_identity_without_cross_source_inheritance() {
    let slot = Arc::new(PriceSnapshotSlot::default());
    slot.install(
        PriceSnapshot::build(
            1,
            vec![],
            vec![
                entry("source-a", "shared-model", "offer-a", 100),
                entry("source-b", "shared-model", "offer-b", 900),
            ],
        )
        .unwrap(),
        None,
    )
    .unwrap();
    let source = source(slot);
    let a = [binding(
        "binding-a",
        "source-a",
        "shared-model",
        "profile-a",
        "offer-a",
    )];
    let b = [binding(
        "binding-b",
        "source-b",
        "shared-model",
        "profile-b",
        "offer-b",
    )];
    let a_quote = source
        .capture_request(WorkspaceId::DEFAULT, &a, 1000)
        .freeze_attempt("binding-a", "shared-model", "profile-a", 2000);
    let b_quote = source
        .capture_request(WorkspaceId::DEFAULT, &b, 1000)
        .freeze_attempt("binding-b", "shared-model", "profile-b", 2000);
    assert_eq!(
        a_quote.quote.unwrap().rates.input_uncached,
        TokenRateV1::known(100)
    );
    assert_eq!(
        b_quote.quote.unwrap().rates.input_uncached,
        TokenRateV1::known(900)
    );

    let mut forged = a[0].clone();
    forged.source_identity_digest = identity("source-b");
    let unknown = source
        .capture_request(WorkspaceId::DEFAULT, &[forged], 1000)
        .freeze_attempt("binding-a", "shared-model", "profile-a", 2000);
    assert_ne!(
        unknown
            .quote
            .as_ref()
            .map(|quote| &quote.rates.input_uncached),
        Some(&TokenRateV1::known(100))
    );
    unknown.validate().unwrap();
}

#[test]
fn stale_offer_mapping_cannot_borrow_current_catalog_price() {
    let slot = Arc::new(PriceSnapshotSlot::default());
    slot.install(
        PriceSnapshot::build(
            1,
            vec![],
            vec![entry("source-a", "model-a", "offer-current", 100)],
        )
        .unwrap(),
        None,
    )
    .unwrap();
    let bindings = [binding(
        "binding",
        "source-a",
        "model-a",
        "profile",
        "offer-old",
    )];
    let evidence = source(slot)
        .capture_request(WorkspaceId::DEFAULT, &bindings, 1000)
        .freeze_attempt("binding", "model-a", "profile", 2000);
    let quote = evidence.quote.as_ref().unwrap();
    assert_eq!(quote.exact_target.source_id, "source-a");
    assert_eq!(quote.actual_offer_ref, None);
    assert_eq!(quote.origin, PriceOriginV1::Unknown);
    assert_eq!(
        quote.rates,
        TokenRatesV1::unknown(PriceUnknownReasonV1::UnverifiedOfferMapping)
    );
    assert_eq!(
        evidence.unknown_reason,
        Some(PriceUnknownReasonV1::UnverifiedOfferMapping)
    );
    evidence.validate().unwrap();
}

#[test]
fn workspace_identity_is_part_of_the_price_target() {
    let slot = Arc::new(PriceSnapshotSlot::default());
    slot.install(
        PriceSnapshot::build(
            1,
            vec![],
            vec![entry("source-a", "model-a", "offer-a", 100)],
        )
        .unwrap(),
        None,
    )
    .unwrap();
    let bindings = [binding(
        "binding", "source-a", "model-a", "profile", "offer-a",
    )];
    let evidence = source(slot)
        .capture_request("personal/other", &bindings, 1000)
        .freeze_attempt("binding", "model-a", "profile", 2000);
    let quote = evidence.quote.unwrap();
    assert_eq!(quote.exact_target.workspace_id.as_str(), "personal/other");
    assert_eq!(quote.origin, PriceOriginV1::Unknown);
    assert_eq!(
        quote.rates.input_uncached,
        TokenRateV1::unknown(PriceUnknownReasonV1::TargetNotInSnapshot)
    );
}

#[test]
fn configured_api_equivalent_price_is_frozen_when_billing_price_is_unavailable() {
    let mut actual = entry("source-a", "model-a", "offer-a", 100);
    actual.catalog_rules.clear();
    actual.actual_offer_ref = None;
    let mut reference = actual.clone();
    reference.target.valuation_kind = PriceValuationKindV1::ApiEquivalent;
    reference.source_override = Some(SourcePriceOverrideV1 {
        target: reference.target.clone(),
        revision: 1,
        setting: SourcePriceSettingV1::Set {
            rates: TokenRatesV1::from_legacy(300_000, 1_200_000),
        },
    });
    let slot = Arc::new(PriceSnapshotSlot::default());
    slot.install(
        PriceSnapshot::build(1, vec![], vec![actual, reference]).unwrap(),
        None,
    )
    .unwrap();
    let evidence = source(slot)
        .capture_request(
            WorkspaceId::DEFAULT,
            &[binding(
                "binding", "source-a", "model-a", "profile", "offer-a",
            )],
            1000,
        )
        .freeze_attempt("binding", "model-a", "profile", 2000);
    let quote = evidence.quote.as_ref().unwrap();
    assert_eq!(quote.valuation_kind, PriceValuationKindV1::ApiEquivalent);
    assert_eq!(quote.origin, PriceOriginV1::Manual);
    assert_eq!(quote.rates.input_uncached, TokenRateV1::known(300_000));
    assert_eq!(
        quote.rates.cache_read,
        TokenRateV1::unknown(PriceUnknownReasonV1::CacheRateNotCollected)
    );
    evidence.validate().unwrap();
}

#[test]
fn configured_reference_does_not_replace_a_verified_billing_price() {
    let actual = entry("source-a", "model-a", "offer-a", 100);
    let mut reference = actual.clone();
    reference.target.valuation_kind = PriceValuationKindV1::ApiEquivalent;
    reference.catalog_rules.clear();
    reference.source_override = Some(SourcePriceOverrideV1 {
        target: reference.target.clone(),
        revision: 1,
        setting: SourcePriceSettingV1::Set {
            rates: TokenRatesV1::from_legacy(900, 900),
        },
    });
    let slot = Arc::new(PriceSnapshotSlot::default());
    slot.install(
        PriceSnapshot::build(1, vec![], vec![actual, reference]).unwrap(),
        None,
    )
    .unwrap();
    let evidence = source(slot)
        .capture_request(
            WorkspaceId::DEFAULT,
            &[binding(
                "binding", "source-a", "model-a", "profile", "offer-a",
            )],
            1000,
        )
        .freeze_attempt("binding", "model-a", "profile", 2000);
    let quote = evidence.quote.unwrap();
    assert_eq!(quote.valuation_kind, PriceValuationKindV1::UsageEstimate);
    assert_eq!(quote.rates.input_uncached, TokenRateV1::known(100));
}

#[test]
fn pinned_publication_keeps_reference_price_after_source_revision_changes() {
    let mut reference = entry("source-a", "current-execution", "offer-a", 2_000_000);
    reference.target.model_identity = PriceModelIdentityV1::LocalModel("authoring-model".into());
    reference.target.valuation_kind = PriceValuationKindV1::ApiEquivalent;
    reference.execution_binding_id = Some("stable-binding".into());
    reference.catalog_rules.clear();
    reference.source_override = Some(SourcePriceOverrideV1 {
        target: reference.target.clone(),
        revision: 2,
        setting: SourcePriceSettingV1::Set {
            rates: TokenRatesV1::from_legacy(2_000_000, 2_000_000),
        },
    });
    let slot = Arc::new(PriceSnapshotSlot::default());
    slot.install(
        PriceSnapshot::build(2, vec![], vec![reference]).unwrap(),
        None,
    )
    .unwrap();
    let bindings = [binding(
        "stable-binding",
        "source-a",
        "pinned-older-execution",
        "profile",
        "older-offer",
    )];
    let captured = source(slot).capture_request(WorkspaceId::DEFAULT, &bindings, 100_000);
    let evidence = captured.freeze_attempt(
        "stable-binding",
        "pinned-older-execution",
        "profile",
        100_001,
    );
    evidence.validate().unwrap();
    let quote = evidence.quote.unwrap();
    assert_eq!(quote.valuation_kind, PriceValuationKindV1::ApiEquivalent);
    assert_eq!(quote.rates.input_uncached, TokenRateV1::known(2_000_000));
    assert_eq!(
        quote.exact_target.model_identity,
        PriceModelIdentityV1::CatalogModel("pinned-older-execution".into())
    );
    assert!(
        captured
            .freeze_attempt("stable-binding", "different-model", "profile", 100_001)
            .quote
            .is_none()
    );
    assert!(
        captured
            .freeze_attempt(
                "other-binding",
                "pinned-older-execution",
                "profile",
                100_001
            )
            .quote
            .is_none()
    );
}
