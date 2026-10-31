//! Real source operation + Secret activation, indexed wallet accounting and restart.
use super::lifecycle::{edit, saved};
use super::*;
use hiroute_application_api::{ComputeKeyBudgetSaveRequestV1, ComputeManagementChangeV3};
use hiroute_domain::*;

fn policy(amount: u64) -> KeyBudgetPolicyV1 {
    KeyBudgetPolicyV1 {
        currency: "USD".into(),
        amount_micros: amount,
        renewal: None,
    }
}
fn configure(stores: &LocalStorageSet, assignment: KeyBudgetAssignmentV1, label: &str) -> String {
    let source = saved(stores);
    let registry = TrustedComputeCandidateRegistry::new();
    let input = ProtectedInput;
    let planner =
        ComputeManagementPlanner::new(&registry, stores.control(), stores.secrets(), &input);
    let preview = planner
        .preview_key_budgets(ComputeKeyBudgetSaveRequestV1 {
            change: ComputeManagementChangeV3 {
                schema: COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V3.into(),
                api_link: None,
                default_preferences: Default::default(),
                change: edit(
                    stores,
                    &source,
                    hiroute_application_api::ComputeManagementEditV1::Rename {
                        display_name: source.display_name.clone(),
                    },
                    &[],
                ),
            },
            network_edit: None,
            key_budget_edits: vec![KeyBudgetEditV1 {
                key: KeyBudgetSelectorV1::Saved {
                    key_id: source.credentials[0].key_id.clone(),
                },
                assignment,
            }],
        })
        .unwrap();
    let runtime = TransactionRuntime::default();
    let external = NoExternal;
    let coordinator = TransactionCoordinator::new(
        stores.control(),
        stores.secrets(),
        stores.runtime(),
        &external,
        &input,
        &runtime,
    );
    coordinator.reconcile_startup_and_open().unwrap();
    apply_preview(
        stores,
        &planner,
        &coordinator,
        &WorkspaceId::default(),
        preview,
        label,
    );
    source.credentials[0].key_id.clone()
}
fn new_budget(amount: u64) -> KeyBudgetAssignmentV1 {
    KeyBudgetAssignmentV1::Configure {
        budget_id: None,
        expected_revision: 0,
        display_name: "Monthly API credits".into(),
        policy: policy(amount),
        initial_remaining_micros: None,
    }
}
fn estimate(source: &str) -> KeySpendEstimateV1 {
    let digest = CanonicalDigest::of_bytes(b"key-budget-test");
    let mut quote = FrozenPriceQuoteV1 {
        generation_ref: None,
        exact_target: PriceTargetV1 {
            workspace_id: WorkspaceId::default(),
            source_id: source.into(),
            source_identity_digest: digest.clone(),
            model_identity: PriceModelIdentityV1::LocalModel("model/one".into()),
            currency: "USD".into(),
            valuation_kind: PriceValuationKindV1::UsageEstimate,
        },
        actual_source_ref: source.into(),
        actual_offer_ref: None,
        reference_model_offer_ref: None,
        valuation_kind: PriceValuationKindV1::UsageEstimate,
        currency: "USD".into(),
        unit: PriceUnitV1::MicrosPerMillionTokens,
        rates: TokenRatesV1 {
            input_uncached: TokenRateV1::known(1_000_000),
            output: TokenRateV1::known(2_000_000),
            cache_read: TokenRateV1::known(100_000),
            cache_write: TokenRateV1::known(1_250_000),
        },
        origin: PriceOriginV1::Manual,
        applied_override_refs: vec![],
        selected_rule_refs: vec![],
        attempt_execution_at: 1,
        applied_schedule: None,
        billing_context: PriceBillingContextV1::StandardTokens,
        unknown_reasons: vec![],
        quote_digest: digest,
    };
    quote.quote_digest = quote.computed_digest().unwrap();
    KeySpendEstimateV1 {
        pricing: ExecutionPricingEvidenceV1 {
            schema_version: EXECUTION_PRICING_SCHEMA_V1.into(),
            request_generation: None,
            captured_at_ms: 1000,
            attempt_execution_at_ms: 1000,
            quote: Some(quote),
            reference_quote: None,
            unknown_reason: None,
            usage_semantics: UsageSemanticsV1 {
                frame_kind: UsageFrameKindV1::Cumulative,
                input: InputUsageMeaningV1::IncludesExclusiveCache,
                output: OutputUsageMeaningV1::IncludesReasoning,
                cache_buckets_exclusive: true,
            },
        },
        input_bound: 100,
        output_bound: 100,
    }
}
fn view(stores: &LocalStorageSet, key: &str) -> KeyBudgetViewV1 {
    let source = saved(stores);
    let id = stores
        .control()
        .key_budget_bindings(&source.source_id)
        .unwrap()[key]
        .clone();
    stores
        .control()
        .key_budget_views(&[id], 1_000)
        .unwrap()
        .remove(0)
}
#[test]
fn key_budget_atomic_save_accounting_reopen_rotation_and_legacy_edits_preserve_spending() {
    let root = tempdir().unwrap();
    populate_saved_source(root.path(), &WorkspaceId::default());
    let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    let before = saved(&stores);
    let key = configure(&stores, new_budget(1_000), "key-budget-create");
    let source = saved(&stores);
    assert_eq!(source.models, before.models);
    assert_eq!(source.credentials, before.credentials);
    let quote = estimate(&source.source_id);
    assert_eq!(quote.upper_bound_micros(), Some(325));
    let reservation = stores
        .control()
        .reserve_key_spend(&key, 1, Some(&quote))
        .unwrap()
        .unwrap();
    stores
        .control()
        .mark_key_spend_sent(&reservation.reservation_id)
        .unwrap();
    let usage = KeySpendSettlementV1::Usage {
        usage: KeySpendUsageV1 {
            input_tokens: Some(10),
            output_tokens: Some(20),
            cache_read_tokens: Some(0),
            cache_write_tokens: Some(0),
            reasoning_tokens: Some(10),
        },
    };
    stores
        .control()
        .settle_key_spend(&reservation.reservation_id, &usage)
        .unwrap();
    stores
        .control()
        .settle_key_spend(&reservation.reservation_id, &usage)
        .unwrap();
    assert_eq!(
        (
            view(&stores, &key).available_micros,
            view(&stores, &key).lifetime_spent_micros
        ),
        (950, 50)
    );
    let current = view(&stores, &key).definition;
    configure(
        &stores,
        KeyBudgetAssignmentV1::Configure {
            budget_id: Some(current.budget_id.clone()),
            expected_revision: current.revision,
            display_name: current.display_name,
            policy: policy(2_000),
            initial_remaining_micros: None,
        },
        "key-budget-increase",
    );
    assert_eq!(view(&stores, &key).available_micros, 1_950);
    // A released V2 rename is still valid and cannot remove a newer Key allowance.
    let source = saved(&stores);
    let registry = TrustedComputeCandidateRegistry::new();
    lifecycle::execute(
        &stores,
        &registry,
        edit(
            &stores,
            &source,
            hiroute_application_api::ComputeManagementEditV1::Rename {
                display_name: "Renamed with allowance".into(),
            },
            &[],
        ),
        "key-budget-legacy-rename",
    );
    let pending = stores
        .control()
        .reserve_key_spend(&key, 1, Some(&quote))
        .unwrap()
        .unwrap();
    stores
        .control()
        .mark_key_spend_sent(&pending.reservation_id)
        .unwrap();
    stores
        .control()
        .settle_key_spend(&pending.reservation_id, &KeySpendSettlementV1::Unknown)
        .unwrap();
    drop(stores);
    let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    assert_eq!(
        (
            view(&stores, &key).available_micros,
            view(&stores, &key).reserved_micros,
            view(&stores, &key).lifetime_spent_micros
        ),
        (1_625, 325, 50)
    );
    assert!(matches!(
        stores.control().reserve_key_spend(&key, 1, None),
        Err(KeyBudgetGateErrorV1::PriceUnavailable)
    ));
    assert!(
        stores
            .control()
            .reserve_key_spend("unlimited-key", 1, None)
            .unwrap()
            .is_none()
    );
    // Unsent work refunds once and never changes historical charges.
    let unsent = stores
        .control()
        .reserve_key_spend(&key, 1, Some(&quote))
        .unwrap()
        .unwrap();
    stores
        .control()
        .settle_key_spend(&unsent.reservation_id, &KeySpendSettlementV1::Unsent)
        .unwrap();
    stores
        .control()
        .settle_key_spend(&unsent.reservation_id, &KeySpendSettlementV1::Unsent)
        .unwrap();
    assert_eq!(view(&stores, &key).available_micros, 1_625);
}

#[test]
fn key_budget_shared_keys_cannot_overbook_and_unknown_prices_never_become_free() {
    let root = tempdir().unwrap();
    populate_saved_source(root.path(), &WorkspaceId::default());
    let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    let key = configure(&stores, new_budget(500), "key-budget-shared");
    let source = saved(&stores);
    let quote = estimate(&source.source_id);
    let first = stores
        .control()
        .reserve_key_spend(&key, 1, Some(&quote))
        .unwrap()
        .unwrap();
    // A second connection holds its own SQLite writer; the existing reservation
    // is visible atomically, independent of observations or process memory.
    let other = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    assert!(matches!(
        other.control().reserve_key_spend(&key, 1, Some(&quote)),
        Err(KeyBudgetGateErrorV1::Exhausted)
    ));
    let mut wrong = quote.clone();
    wrong.pricing.quote.as_mut().unwrap().currency = "CNY".into();
    assert!(matches!(
        other.control().reserve_key_spend(&key, 1, Some(&wrong)),
        Err(KeyBudgetGateErrorV1::CurrencyMismatch)
    ));
    let mut unknown = quote.clone();
    let q = unknown.pricing.quote.as_mut().unwrap();
    q.rates.cache_write = TokenRateV1::unknown(PriceUnknownReasonV1::CacheRateNotCollected);
    q.quote_digest = q.computed_digest().unwrap();
    assert!(matches!(
        other.control().reserve_key_spend(&key, 1, Some(&unknown)),
        Err(KeyBudgetGateErrorV1::PriceUnavailable)
    ));
    other
        .control()
        .settle_key_spend(&first.reservation_id, &KeySpendSettlementV1::Unsent)
        .unwrap();
    assert_eq!(view(&stores, &key).available_micros, 500);
}
