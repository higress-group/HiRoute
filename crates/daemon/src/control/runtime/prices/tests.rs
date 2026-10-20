use super::*;
use hiroute_application::ApplicationService;
use hiroute_application_api::*;
use hiroute_domain::*;
use hiroute_integrations::RegisteredComputeDiscoveryFactV1;
use serde_json::json;

thread_local! { static FAIL_INSTALL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
pub(super) fn take_install_failure() -> bool {
    FAIL_INSTALL.with(|f| f.replace(false))
}

fn subscription_catalog() -> TrustedReleaseCatalog {
    const MANIFEST: &[u8] =
        include_bytes!("../../../../../../assets/release-facts/current/bundle/manifest.json");
    const REGISTRY: &[u8] = include_bytes!(
        "../../../../../../assets/release-facts/current/bundle/connector-registry.json"
    );
    const MODEL_DATA: &[u8] =
        include_bytes!("../../../../../../assets/release-facts/current/bundle/model-data.json");
    TrustedReleaseCatalog::load_bundled_release_facts(MANIFEST, MANIFEST, REGISTRY, MODEL_DATA)
        .unwrap()
}

fn paid_catalog() -> TrustedReleaseCatalog {
    let mut registry: ConnectorRegistryBundleV1 = serde_json::from_slice(include_bytes!(
        "../../../../../../assets/release-facts/current/bundle/connector-registry.json"
    ))
    .unwrap();
    let mut model_data: ReleaseModelDataBundleV2 = serde_json::from_slice(include_bytes!(
        "../../../../../../assets/release-facts/current/bundle/model-data.json"
    ))
    .unwrap();
    registry
        .connection_options
        .iter_mut()
        .find(|option| option.connection_option_id == "zhipu.coding-plan.cn.v1")
        .unwrap()
        .billing_class = BillingClass::Paid;
    model_data
        .data
        .offers
        .iter_mut()
        .find(|offer| offer.offer_id == "offer.zhipu.coding-plan")
        .unwrap()
        .billing_class = BillingClass::Paid;
    registry.validate().unwrap();
    model_data.validate_against(&registry).unwrap();
    let registry_bytes = serde_json::to_vec(&registry).unwrap();
    let model_data_bytes = serde_json::to_vec(&model_data).unwrap();
    let manifest = ReleaseFactsManifestV2 {
        schema: RELEASE_FACTS_SCHEMA_V2.into(),
        tool_version: RELEASE_FACTS_TOOL_VERSION_V2.into(),
        catalog_id: "fixture/price-edit-paid".into(),
        product_release: registry.product_release.clone(),
        sequence: 1,
        connector_registry_digest: CanonicalDigest::of_bytes(&registry_bytes),
        model_data_digest: CanonicalDigest::of_bytes(&model_data_bytes),
        cross_reference_digest: model_data.cross_reference_digest(&registry).unwrap(),
    };
    let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
    TrustedReleaseCatalog::load_bundled_release_facts(
        &manifest_bytes,
        &manifest_bytes,
        &registry_bytes,
        &model_data_bytes,
    )
    .unwrap()
}

fn seed_source(runtime: &ProductionControlRuntime, billing: BillingClass) -> SourceBindingV1 {
    let (option, endpoint, base_url, upstream_model, model_configuration) = match billing {
        BillingClass::Subscription => (
            "zhipu.coding-plan.cn.v1",
            "endpoint.zhipu.coding-plan.cn.v1",
            "https://open.bigmodel.cn/api/anthropic",
            "glm-5.3",
            "model.zhipu.glm-5.3",
        ),
        BillingClass::Paid => (
            "zhipu.coding-plan.cn.v1",
            "endpoint.zhipu.coding-plan.cn.v1",
            "https://open.bigmodel.cn/api/anthropic",
            "glm-5.3",
            "model.zhipu.glm-5.3",
        ),
        _ => panic!("unsupported price test billing class"),
    };
    let c = runtime.adapter.current_catalog().unwrap();
    let candidate = c
        .authorize_compute_discovery(RegisteredComputeDiscoveryFactV1 {
            agent_id: "agent.claude-code".into(),
            scanner_id: "scanner.claude.v1".into(),
            scanner_version: "1".into(),
            discovered_source_ref: "claude/settings/fixture".into(),
            configuration_revision: 1,
            connection_option_id: option.into(),
            endpoint_profile_id: endpoint.into(),
            endpoint_profile_revision: 1,
            registered_base_url: base_url.into(),
            observed_model_id: upstream_model.into(),
            model_configuration_id: model_configuration.into(),
            protected_credential_available: true,
        })
        .unwrap();
    let prepared = c
        .prepare_compute_projection(
            candidate,
            ComputeProjectionExpectationV1 {
                source_revision: 0,
                source_digest: None,
                binding_revision: 0,
                binding_digest: None,
                inventory_revision: 0,
                inventory_digest: None,
            },
            true,
        )
        .unwrap();
    let spec = ChangeSpecV1 {
        schema_version: CHANGE_SPEC_SCHEMA_V1,
        command_id: "compute.connection.apply".into(),
        resource_id: Some(prepared.desired.source.source_id.clone()),
        desired_state: json!({"connection_option_id": prepared.desired.source.connection_option_id,"source_id":prepared.desired.source.source_id,"explicit_materialization":true,"expected_source_revision":0,"projection":prepared}),
    };
    let plan = TransactionPlanV1::from_compute_projection_planner(
        spec,
        None,
        prepared.clone(),
        c.registry(),
        true,
    )
    .unwrap();
    let stores = runtime.adapter.stores.lock().unwrap();
    let rev = stores
        .control()
        .current_revisions(&WorkspaceId::default())
        .unwrap()
        .target;
    let effect = stores
        .control()
        .apply_compute_source(
            &OperationId::parse("op_41414141414141414141414141414141").unwrap(),
            &WorkspaceId::default(),
            rev,
            &plan.compute_source().unwrap(),
        )
        .unwrap();
    stores.control().activate_control(&effect).unwrap();
    drop(stores);
    runtime.adapter.rebuild_price_snapshot().unwrap();
    prepared.desired.binding
}
fn wire(operation: &str, payload: serde_json::Value) -> LocalControlRequestV2 {
    LocalControlRequestV2 {
        schema_version: LOCAL_CONTROL_SCHEMA_V2,
        request_id: "price-test".into(),
        principal: PrincipalV1::interactive_user(),
        operation_id: operation.into(),
        payload,
        protected_grant: None,
    }
}

#[test]
fn subscription_usage_is_non_editable_but_api_reference_prices_are_editable() {
    #[cfg(unix)]
    if crate::test_support::isolated_agent_home(
        "control::runtime::prices::tests::subscription_usage_is_non_editable_but_api_reference_prices_are_editable",
    ) {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let runtime = ProductionControlRuntime::open_with_release_catalog(
        dir.path().join("storage"),
        subscription_catalog(),
    )
    .unwrap();
    let binding = seed_source(&runtime, BillingClass::Subscription);
    let app = ApplicationService::new(runtime.application_ports());
    for (kind, editable) in [
        (PriceValuationKindV1::ApiEquivalent, true),
        (PriceValuationKindV1::UsageEstimate, false),
    ] {
        let response = app.dispatch(wire(
            "GetEffectivePrices",
            serde_json::to_value(GetEffectivePricesV2 {
                targets: vec![EffectivePriceTargetQueryV2 {
                    query_id: "subscription-price".into(),
                    target_locator: PriceTargetLocatorV1::Binding {
                        binding_id: binding.binding_id.clone(),
                    },
                    currency: "USD".into(),
                    valuation_kind: kind,
                }],
            })
            .unwrap(),
        ));
        assert!(response.error.is_none(), "{response:?}");
        let response: EffectivePricesResultV2 =
            serde_json::from_value(response.data.unwrap()).unwrap();
        assert_eq!(response.items[0].edit_context.is_some(), editable);
    }
}

#[test]
fn source_prices_application_apply_replays_restarts_and_never_waits_for_store_on_freeze() {
    #[cfg(unix)]
    if crate::test_support::isolated_agent_home(
        "control::runtime::prices::tests::source_prices_application_apply_replays_restarts_and_never_waits_for_store_on_freeze",
    ) {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("storage");
    let runtime =
        ProductionControlRuntime::open_with_release_catalog(&root, paid_catalog()).unwrap();
    let binding = seed_source(&runtime, BillingClass::Paid);
    let app = ApplicationService::new(runtime.application_ports());
    let effective = app.dispatch(wire(
        "GetEffectivePrices",
        serde_json::to_value(GetEffectivePricesV2 {
            targets: vec![EffectivePriceTargetQueryV2 {
                query_id: "price-edit-context".into(),
                target_locator: PriceTargetLocatorV1::Binding {
                    binding_id: binding.binding_id.clone(),
                },
                currency: "USD".into(),
                valuation_kind: PriceValuationKindV1::UsageEstimate,
            }],
        })
        .unwrap(),
    ));
    assert!(effective.error.is_none(), "{effective:?}");
    let effective: EffectivePricesResultV2 =
        serde_json::from_value(effective.data.unwrap()).unwrap();
    let initial_edit_context = effective.items[0].edit_context.clone().unwrap();
    assert_eq!(
        initial_edit_context.target_locator,
        PriceTargetLocatorV1::Binding {
            binding_id: binding.binding_id.clone()
        }
    );
    assert_eq!(
        initial_edit_context.expected_source_revision,
        binding.source_revision
    );
    assert_eq!(
        initial_edit_context.expected_binding_revision,
        binding.revision
    );
    assert_eq!(initial_edit_context.expected_override_revision, 0);
    let change = PreviewPriceOverrideChangeV2 {
        target_locator: initial_edit_context.target_locator.clone(),
        currency: "USD".into(),
        valuation_kind: PriceValuationKindV1::UsageEstimate,
        action: SourcePriceSettingV1::Set {
            rates: TokenRatesV1::from_legacy(1_200_000, 4_800_000),
        },
        expected_source_revision: initial_edit_context.expected_source_revision,
        expected_binding_revision: Some(initial_edit_context.expected_binding_revision),
        expected_override_revision: initial_edit_context.expected_override_revision,
    };
    let response = app.dispatch(wire(
        "PreviewPriceOverrideChange",
        serde_json::to_value(change).unwrap(),
    ));
    assert!(response.error.is_none(), "{response:?}");
    let preview: PriceOverridePreviewV2 = serde_json::from_value(response.data.unwrap()).unwrap();
    assert!(
        runtime
            .adapter
            .stores
            .lock()
            .unwrap()
            .control()
            .source_price_override(&preview.normalized_target)
            .unwrap()
            .is_none()
    );
    let payload = ApplyPriceOverrideChangeV2 {
        spec: preview.spec,
        accept_digest: preview.change_digest,
        expected_revisions: preview.expected_revisions,
        idempotency_key: "price-test-one".into(),
    };
    let apply = wire(
        "ApplyPriceOverrideChange",
        serde_json::to_value(payload).unwrap(),
    );
    let mut with_grant = apply.clone();
    with_grant.protected_grant = Some(ProtectedClientGrantV2 {
        principal_kind: PrincipalKind::InteractiveUser,
        capability: "obsolete-price-grant".into(),
    });
    let denied = app.dispatch(with_grant);
    assert_eq!(denied.error.unwrap().code, ErrorCode::CapabilityDenied);
    let result = app.dispatch(apply.clone());
    assert!(result.error.is_none(), "{result:?}");
    assert_eq!(result.data.as_ref().unwrap()["state"], "succeeded");
    assert_eq!(
        result.operation.as_ref().unwrap().operation_id,
        result.data.as_ref().unwrap()["operation_id"]
            .as_str()
            .unwrap()
    );
    assert_eq!(result.data, app.dispatch(apply).data);
    let stale = app.dispatch(wire(
        "PreviewPriceOverrideChange",
        serde_json::to_value(PreviewPriceOverrideChangeV2 {
            target_locator: initial_edit_context.target_locator,
            currency: "USD".into(),
            valuation_kind: PriceValuationKindV1::UsageEstimate,
            action: SourcePriceSettingV1::FollowCatalog,
            expected_source_revision: initial_edit_context.expected_source_revision,
            expected_binding_revision: Some(initial_edit_context.expected_binding_revision),
            expected_override_revision: initial_edit_context.expected_override_revision,
        })
        .unwrap(),
    ));
    assert_eq!(stale.error.unwrap().code, ErrorCode::RevisionConflict);
    let effective = app.dispatch(wire(
        "GetEffectivePrices",
        serde_json::to_value(GetEffectivePricesV2 {
            targets: vec![EffectivePriceTargetQueryV2 {
                query_id: "price-edit-context-refreshed".into(),
                target_locator: PriceTargetLocatorV1::Binding {
                    binding_id: binding.binding_id.clone(),
                },
                currency: "USD".into(),
                valuation_kind: PriceValuationKindV1::UsageEstimate,
            }],
        })
        .unwrap(),
    ));
    let refreshed: EffectivePricesResultV2 =
        serde_json::from_value(effective.data.unwrap()).unwrap();
    assert_eq!(
        refreshed.items[0]
            .edit_context
            .as_ref()
            .unwrap()
            .expected_override_revision,
        1
    );
    let slot = runtime.price_snapshot_slot();
    let handle = slot.capture_current_price_snapshot();
    let target = preview.normalized_target;
    let locked = runtime.adapter.stores.lock().unwrap();
    let target_copy = target.clone();
    let quote = std::thread::spawn(move || {
        handle
            .freeze_price(&target_copy, 100, PriceBillingContextV1::StandardTokens)
            .unwrap()
    })
    .join()
    .unwrap();
    assert_eq!(quote.origin, PriceOriginV1::Manual);
    quote.verify_digest().unwrap();
    drop(locked);
    // Inject a failure after SQLite commit, before pointer installation. The operation must
    // remain recoverable, and a new writer must not bypass the unfinished installation.
    let second = PreviewPriceOverrideChangeV2 {
        target_locator: PriceTargetLocatorV1::Binding {
            binding_id: binding.binding_id.clone(),
        },
        currency: "USD".into(),
        valuation_kind: PriceValuationKindV1::UsageEstimate,
        action: SourcePriceSettingV1::Set {
            rates: TokenRatesV1::from_legacy(2_000_000, 6_000_000),
        },
        expected_source_revision: binding.source_revision,
        expected_binding_revision: Some(binding.revision),
        expected_override_revision: 1,
    };
    let r = app.dispatch(wire(
        "PreviewPriceOverrideChange",
        serde_json::to_value(second).unwrap(),
    ));
    let second: PriceOverridePreviewV2 = serde_json::from_value(r.data.unwrap()).unwrap();
    let request = wire(
        "ApplyPriceOverrideChange",
        serde_json::to_value(ApplyPriceOverrideChangeV2 {
            spec: second.spec,
            accept_digest: second.change_digest,
            expected_revisions: second.expected_revisions,
            idempotency_key: "price-recovery-two".into(),
        })
        .unwrap(),
    );
    FAIL_INSTALL.with(|f| f.set(true));
    let failed = app.dispatch(request);
    assert!(failed.error.is_some());
    let pending = runtime
        .adapter
        .stores
        .lock()
        .unwrap()
        .control()
        .recoverable_operations()
        .unwrap();
    assert_eq!(pending.len(), 1);
    assert_ne!(pending[0].state, OperationState::Succeeded);
    let still_old = slot
        .capture_current_price_snapshot()
        .freeze_price(&target, 100, PriceBillingContextV1::StandardTokens)
        .unwrap();
    assert_eq!(still_old.rates, quote.rates);
    drop(app);
    drop(runtime);
    let runtime =
        ProductionControlRuntime::open_with_release_catalog(&root, paid_catalog()).unwrap();
    let quote2 = runtime
        .price_snapshot_slot()
        .capture_current_price_snapshot()
        .freeze_price(&target, 100, PriceBillingContextV1::StandardTokens)
        .unwrap();
    assert_eq!(quote2.rates.input_uncached, TokenRateV1::known(2_000_000));
    assert_ne!(quote.generation_ref, quote2.generation_ref);
    assert_eq!(
        slot.capture_current_price_snapshot()
            .freeze_price(&target, 100, PriceBillingContextV1::StandardTokens)
            .unwrap()
            .rates,
        quote.rates
    );
    assert!(
        runtime
            .adapter
            .stores
            .lock()
            .unwrap()
            .control()
            .recoverable_operations()
            .unwrap()
            .is_empty()
    );
}
