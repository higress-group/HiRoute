use super::*;
use hiroute_domain::*;

fn entry(source: &str) -> PriceIndexEntryV1 {
    PriceIndexEntryV1 {
        execution_binding_id: None,
        target: PriceTargetV1 {
            workspace_id: WorkspaceId::default(),
            source_id: source.into(),
            source_identity_digest: CanonicalDigest::of_bytes(source.as_bytes()),
            model_identity: PriceModelIdentityV1::CatalogModel("test-model".into()),
            currency: "USD".into(),
            valuation_kind: PriceValuationKindV1::UsageEstimate,
        },
        actual_offer_ref: Some("test-offer".into()),
        reference_model_offer_ref: None,
        catalog_rules: vec![CatalogPriceRuleV2 {
            rule_ref: PriceFactRefV1 {
                id: "base".into(),
                revision: 1,
                digest: CanonicalDigest::of_bytes(b"base"),
            },
            rates: TokenRatesV1::from_legacy(1_200_000, 4_800_000),
            schedule: None,
        }],
        legacy_overrides: vec![],
        source_override: None,
    }
}
fn freeze(handle: &PriceSnapshotHandle, target: &PriceTargetV1, at: i64) -> FrozenPriceQuoteV1 {
    handle
        .freeze_price(target, at, PriceBillingContextV1::StandardTokens)
        .unwrap()
}

#[test]
fn compiled_custom_identity_uses_only_its_generation_bound_source_override() {
    let mut custom = entry("custom-source");
    custom.target.model_identity = PriceModelIdentityV1::LocalModel("model/authoring-ref".into());
    custom.execution_binding_id = Some("binding-local".into());
    custom.catalog_rules.clear();
    custom.actual_offer_ref = None;
    custom.source_override = Some(SourcePriceOverrideV1 {
        target: custom.target.clone(),
        revision: 1,
        setting: SourcePriceSettingV1::Set {
            rates: TokenRatesV1::from_legacy(300_000, 1_200_000),
        },
    });
    let slot = PriceSnapshotSlot::default();
    slot.install(
        PriceSnapshot::build(1, vec![], vec![custom.clone()]).unwrap(),
        None,
    )
    .unwrap();
    let handle = slot.capture_current_price_snapshot();
    let mut executed = custom.target.clone();
    executed.model_identity =
        PriceModelIdentityV1::CatalogModel("model/source-local/executed".into());
    let read = |target: &PriceTargetV1| {
        handle
            .freeze_execution_price(
                target,
                "binding-local",
                100,
                PriceBillingContextV1::StandardTokens,
            )
            .unwrap()
    };
    let quote = read(&executed);
    assert_eq!(quote.rates.input_uncached, TokenRateV1::known(300_000));
    assert_eq!(quote.exact_target, executed);
    assert_eq!(
        quote.applied_override_refs[0].id,
        custom.target.digest().unwrap().to_string()
    );
    quote.verify_digest().unwrap();
    for changed in ["source", "digest", "currency", "basis"] {
        let mut wrong = executed.clone();
        match changed {
            "source" => wrong.source_id = "another-source".into(),
            "digest" => wrong.source_identity_digest = CanonicalDigest::of_bytes(b"replaced"),
            "currency" => wrong.currency = "EUR".into(),
            _ => wrong.valuation_kind = PriceValuationKindV1::ApiEquivalent,
        }
        assert_eq!(read(&wrong).origin, PriceOriginV1::Unknown, "{changed}");
    }
    let mut newer_execution = executed.clone();
    newer_execution.model_identity =
        PriceModelIdentityV1::CatalogModel("model/source-local/new-revision".into());
    assert_eq!(
        read(&newer_execution).rates.input_uncached,
        TokenRateV1::known(300_000)
    );
    assert_eq!(
        handle
            .freeze_execution_price(
                &executed,
                "other-binding",
                100,
                PriceBillingContextV1::StandardTokens
            )
            .unwrap()
            .origin,
        PriceOriginV1::Unknown
    );
    let mut duplicate = custom.clone();
    duplicate.target.model_identity =
        PriceModelIdentityV1::LocalModel("model/other-authoring-ref".into());
    duplicate.source_override.as_mut().unwrap().target = duplicate.target.clone();
    slot.install(
        PriceSnapshot::build(2, vec![], vec![custom, duplicate]).unwrap(),
        handle.generation_ref(),
    )
    .unwrap();
    let ambiguous = slot
        .capture_current_price_snapshot()
        .freeze_execution_price(
            &executed,
            "binding-local",
            100,
            PriceBillingContextV1::StandardTokens,
        )
        .unwrap();
    assert_eq!(ambiguous.origin, PriceOriginV1::Unknown);
    assert_eq!(
        ambiguous.unknown_reasons,
        vec![PriceUnknownReasonV1::UnverifiedOfferMapping]
    );
    assert!(ambiguous.generation_ref.is_some());
    ambiguous.verify_digest().unwrap();
    assert_eq!(
        read(&executed).rates.input_uncached,
        TokenRateV1::known(300_000)
    );
}
#[test]
fn price_snapshot_manual_without_catalog_is_source_currency_and_identity_isolated() {
    let mut a = entry("a");
    let b = entry("b");
    a.catalog_rules.clear();
    a.actual_offer_ref = None;
    a.source_override = Some(SourcePriceOverrideV1 {
        target: a.target.clone(),
        revision: 1,
        setting: SourcePriceSettingV1::Set {
            rates: TokenRatesV1::from_legacy(2_000_000, 6_000_000),
        },
    });
    let slot = PriceSnapshotSlot::default();
    slot.install(
        PriceSnapshot::build(1, vec![], vec![a.clone(), b.clone()]).unwrap(),
        None,
    )
    .unwrap();
    let h = slot.capture_current_price_snapshot();
    let quote = freeze(&h, &a.target, 100);
    assert_eq!(quote.origin, PriceOriginV1::Manual);
    assert_eq!(quote.rates.input_uncached, TokenRateV1::known(2_000_000));
    assert!(quote.selected_rule_refs.is_empty());
    quote.verify_digest().unwrap();
    assert_eq!(
        freeze(&h, &b.target, 100).rates.input_uncached,
        TokenRateV1::known(1_200_000)
    );
    let mut other = a.target.clone();
    other.currency = "CNY".into();
    assert_eq!(freeze(&h, &other, 100).origin, PriceOriginV1::Unknown);
    other = a.target.clone();
    other.source_identity_digest = CanonicalDigest::of_bytes(b"changed-identity");
    assert_eq!(freeze(&h, &other, 100).origin, PriceOriginV1::Unknown);
    other = a.target.clone();
    other.valuation_kind = PriceValuationKindV1::ApiEquivalent;
    assert_eq!(freeze(&h, &other, 100).origin, PriceOriginV1::Unknown);
}
#[test]
fn price_snapshot_request_keeps_generation_but_attempts_use_their_own_time() {
    let mut a = entry("a");
    a.catalog_rules.push(CatalogPriceRuleV2 {
        rule_ref: PriceFactRefV1 {
            id: "night".into(),
            revision: 1,
            digest: CanonicalDigest::of_bytes(b"night"),
        },
        rates: TokenRatesV1::from_legacy(300_000, 800_000),
        schedule: Some(PriceScheduleV1 {
            timezone_id: "UTC".into(),
            timezone_data_revision: "test".into(),
            schedule_compile_version: "v1".into(),
            effective_from: 0,
            effective_until: 1_000_000,
            utc_offset_minutes: 0,
            weekdays: (1..=7).collect(),
            start_minute: 0,
            end_minute: 60,
        }),
    });
    let slot = PriceSnapshotSlot::default();
    slot.install(
        PriceSnapshot::build(1, vec![], vec![a.clone()]).unwrap(),
        None,
    )
    .unwrap();
    let r1 = slot.capture_current_price_snapshot();
    let before = freeze(&r1, &a.target, 86_399);
    a.source_override = Some(SourcePriceOverrideV1 {
        target: a.target.clone(),
        revision: 1,
        setting: SourcePriceSettingV1::Set {
            rates: TokenRatesV1::from_legacy(2_000_000, 6_000_000),
        },
    });
    slot.install(
        PriceSnapshot::build(2, vec![], vec![a.clone()]).unwrap(),
        r1.generation_ref(),
    )
    .unwrap();
    let after = freeze(&r1, &a.target, 86_400);
    assert_eq!(before.generation_ref, after.generation_ref);
    assert_eq!(before.rates.input_uncached, TokenRateV1::known(1_200_000));
    assert_eq!(after.rates.input_uncached, TokenRateV1::known(300_000));
    let r2 = slot.capture_current_price_snapshot();
    assert_ne!(r1.generation_ref(), r2.generation_ref());
    assert_eq!(
        freeze(&r2, &a.target, 86_400).rates.input_uncached,
        TokenRateV1::known(2_000_000)
    );
    assert!(
        slot.install(
            PriceSnapshot::build(1, vec![], vec![a]).unwrap(),
            r1.generation_ref()
        )
        .is_err()
    );
    before.verify_digest().unwrap();
    after.verify_digest().unwrap();
}
#[test]
fn price_snapshot_restore_skips_legacy_and_keeps_unknown_cache() {
    let mut a = entry("a");
    a.legacy_overrides.push(PriceOverrideV1 {
        override_id: "legacy".into(),
        revision: 1,
        offer_ref: "test-offer".into(),
        model_configuration_id: "test-model".into(),
        currency: "USD".into(),
        operation: PriceOverrideOperation::Replace,
        target_rule_id: None,
        input_value: Some(900_000),
        output_value: Some(900_000),
    });
    let slot = PriceSnapshotSlot::default();
    slot.install(
        PriceSnapshot::build(1, vec![], vec![a.clone()]).unwrap(),
        None,
    )
    .unwrap();
    let old = slot.capture_current_price_snapshot();
    assert_eq!(
        freeze(&old, &a.target, 10).origin,
        PriceOriginV1::LegacyOverride
    );
    a.source_override = Some(SourcePriceOverrideV1 {
        target: a.target.clone(),
        revision: 1,
        setting: SourcePriceSettingV1::FollowCatalog,
    });
    slot.install(
        PriceSnapshot::build(2, vec![], vec![a.clone()]).unwrap(),
        old.generation_ref(),
    )
    .unwrap();
    let quote = freeze(&slot.capture_current_price_snapshot(), &a.target, 10);
    assert_eq!(quote.origin, PriceOriginV1::Catalog);
    assert_eq!(quote.rates.input_uncached, TokenRateV1::known(1_200_000));
    assert_eq!(
        quote.rates.cache_read,
        TokenRateV1::unknown(PriceUnknownReasonV1::CacheRateNotCollected)
    );
}
#[test]
fn price_snapshot_unavailable_capture_does_not_later_adopt_a_generation() {
    let a = entry("a");
    let slot = PriceSnapshotSlot::default();
    let unavailable = slot.capture_current_price_snapshot();
    slot.install(
        PriceSnapshot::build(1, vec![], vec![a.clone()]).unwrap(),
        None,
    )
    .unwrap();
    let quote = freeze(&unavailable, &a.target, 10);
    assert!(quote.generation_ref.is_none());
    assert_eq!(
        quote.unknown_reasons,
        vec![PriceUnknownReasonV1::SnapshotUnavailable]
    );
    let quote = slot
        .capture_current_price_snapshot()
        .freeze_price(
            &a.target,
            10,
            PriceBillingContextV1::Unsupported {
                condition: "cache-ttl".into(),
            },
        )
        .unwrap();
    assert_eq!(
        quote.unknown_reasons,
        vec![PriceUnknownReasonV1::UnsupportedBillingCondition]
    );
}

#[test]
fn price_snapshot_disabled_base_cannot_be_bypassed_by_a_matching_schedule() {
    let mut a = entry("a");
    let mut scheduled = a.catalog_rules[0].clone();
    scheduled.rule_ref.id = "night".into();
    scheduled.schedule = Some(PriceScheduleV1 {
        timezone_id: "UTC".into(),
        timezone_data_revision: "test".into(),
        schedule_compile_version: "v1".into(),
        effective_from: 0,
        effective_until: 1_000_000,
        utc_offset_minutes: 0,
        weekdays: (1..=7).collect(),
        start_minute: 0,
        end_minute: 60,
    });
    a.catalog_rules.push(scheduled);
    a.legacy_overrides.push(PriceOverrideV1 {
        override_id: "disable-base".into(),
        revision: 1,
        offer_ref: "test-offer".into(),
        model_configuration_id: "test-model".into(),
        currency: "USD".into(),
        operation: PriceOverrideOperation::DisableCatalogRule,
        target_rule_id: Some("base".into()),
        input_value: None,
        output_value: None,
    });
    let slot = PriceSnapshotSlot::default();
    slot.install(
        PriceSnapshot::build(1, vec![], vec![a.clone()]).unwrap(),
        None,
    )
    .unwrap();
    let quote = freeze(&slot.capture_current_price_snapshot(), &a.target, 10);
    assert_eq!(quote.origin, PriceOriginV1::Unknown);
    assert_eq!(
        quote.rates.input_uncached,
        TokenRateV1::unknown(PriceUnknownReasonV1::DisabledRule)
    );
    assert!(quote.selected_rule_refs.is_empty());
}

#[test]
fn price_snapshot_rejects_unbounded_or_malformed_frozen_evidence() {
    let a = entry("a");
    let invalid: CanonicalDigest = serde_json::from_str("\"not-a-digest\"").unwrap();
    let mut bad = a.clone();
    bad.target.source_identity_digest = invalid.clone();
    assert!(PriceSnapshot::build(1, vec![], vec![bad]).is_err());
    let mut bad = a.clone();
    bad.catalog_rules[0].rule_ref.digest = invalid;
    assert!(PriceSnapshot::build(1, vec![], vec![bad]).is_err());
    let mut bad = a.clone();
    bad.actual_offer_ref = Some("x".repeat(257));
    assert!(PriceSnapshot::build(1, vec![], vec![bad]).is_err());
    let r = a.catalog_rules[0].rule_ref.clone();
    assert!(PriceSnapshot::build(1, vec![r.clone(), r], vec![a]).is_err());
}
