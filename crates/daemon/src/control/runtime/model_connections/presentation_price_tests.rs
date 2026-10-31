//! A saved paid source's explicit price target remains editable without catalog rates.
use super::*;
use hiroute_application::ApplicationService;
use hiroute_application::control::ComputeManagementControlPort;
use hiroute_application_api::*;
use hiroute_domain::{
    PriceOriginV1, PriceUnknownReasonV1, PriceValuationKindV1, ProtectedSecret,
    SourcePriceSettingV1, TokenRatesV1,
};
use hiroute_integrations::{
    ModelConnectionProbeCredentialV1, ModelDirectoryHttpResponseV1, ModelDirectoryTransportErrorV1,
    ModelDirectoryTransportV1, NormalizedModelConnectionTargetV1,
};
use serde::de::DeserializeOwned;
use serde_json::json;

struct Directory(String);
impl ModelDirectoryTransportV1 for Directory {
    fn get(
        &self,
        _: &NormalizedModelConnectionTargetV1,
        _: Option<&str>,
        _: Option<ModelConnectionProbeCredentialV1<'_>>,
        _: std::time::Duration,
        _: usize,
    ) -> Result<ModelDirectoryHttpResponseV1, ModelDirectoryTransportErrorV1> {
        Ok(ModelDirectoryHttpResponseV1 {
            status: 200,
            body: serde_json::to_vec(&json!({"data":[{"id":self.0}]})).unwrap(),
            truncated: false,
        })
    }
}

fn user<T>(value: T) -> NativeUserFactValueV1<T> {
    NativeUserFactValueV1 {
        value: Some(value),
        basis: NativeUserFactBasisV1::UserDeclared,
    }
}

fn dispatch<T: DeserializeOwned>(
    app: &ApplicationService,
    operation: &str,
    payload: impl serde::Serialize,
) -> T {
    let response = app.dispatch(LocalControlRequestV2 {
        schema_version: LOCAL_CONTROL_SCHEMA_V2,
        request_id: "price-presentation-test".into(),
        principal: PrincipalV1::interactive_user(),
        operation_id: operation.into(),
        payload: serde_json::to_value(payload).unwrap(),
        protected_grant: None,
    });
    assert!(response.error.is_none(), "{response:?}");
    serde_json::from_value(response.data.unwrap()).unwrap()
}

fn effective(app: &ApplicationService, binding_id: &str) -> EffectivePricesResultV2 {
    dispatch(
        app,
        "GetEffectivePrices",
        GetEffectivePricesV2 {
            targets: vec![EffectivePriceTargetQueryV2 {
                query_id: "selected".into(),
                target_locator: PriceTargetLocatorV1::Binding {
                    binding_id: binding_id.into(),
                },
                currency: "USD".into(),
                valuation_kind: PriceValuationKindV1::ApiEquivalent,
            }],
        },
    )
}

fn apply_setting(
    app: &ApplicationService,
    binding_id: &str,
    setting: SourcePriceSettingV1,
    key: &str,
) {
    let edit = effective(app, binding_id).items[0]
        .edit_context
        .clone()
        .unwrap();
    let preview: PriceOverridePreviewV2 = dispatch(
        app,
        "PreviewPriceOverrideChange",
        PreviewPriceOverrideChangeV2 {
            target_locator: edit.target_locator,
            currency: "USD".into(),
            valuation_kind: PriceValuationKindV1::ApiEquivalent,
            expected_source_revision: edit.expected_source_revision,
            expected_binding_revision: Some(edit.expected_binding_revision),
            expected_override_revision: edit.expected_override_revision,
            action: setting,
        },
    );
    let result: serde_json::Value = dispatch(
        app,
        "ApplyPriceOverrideChange",
        ApplyPriceOverrideChangeV2 {
            spec: preview.spec,
            accept_digest: preview.change_digest,
            expected_revisions: preview.expected_revisions,
            idempotency_key: key.into(),
        },
    );
    assert_eq!(result["state"], "succeeded");
}

fn saved_api(
    option: &str,
    model_id: &str,
) -> (
    crate::test_support::PrivateTempDir,
    crate::control::runtime::ProductionControlRuntime,
    String,
) {
    let catalog = crate::release_catalog::current_fixture_catalog();
    let root = crate::test_support::private_tempdir();
    let provenance = catalog.compute_catalog_provenance().unwrap();
    let runtime = crate::control::runtime::ProductionControlRuntime::open_with_release_catalog_and_model_transport(
        root.path(),
        catalog,
        std::sync::Arc::new(Directory(model_id.into())),
    )
    .unwrap();
    let request = RegisteredModelConnectionCheckRequestV1 {
        inference_model_id: None,
        models: vec![NativeUserModelDeclarationV1 {
            upstream_model_id: model_id.into(),
            display_name: "Fixture model".into(),
            catalog_configuration_id: None,
            membership: ComputeModelMembershipV2::UserDeclared,
            capabilities: NativeUserCapabilityDeclarationV1 {
                tool: user(true),
                vision: user(false),
                streaming: user(true),
                context_tokens: user(100_000),
                max_output_tokens: user(4_096),
                native_reasoning: user(hiroute_domain::NativeReasoningCapabilityV1::Fixed {
                    profile: "provider-default".into(),
                }),
            },
        }],
        connection_option_id: option.into(),
        display_name: Some("Fixture paid API".into()),
        expected_catalog: ComputeCatalogProvenanceViewV1 {
            product_release: provenance.product_release,
            catalog_binding_id: provenance.catalog_binding_id,
            release_sequence: provenance.release_sequence,
            connector_registry_digest: provenance.connector_registry_digest,
            model_data_digest: provenance.model_data_digest,
            cross_reference_digest: provenance.cross_reference_digest,
        },
        candidate_ref: None,
        lineage_ref: "lineage/price-presentation".into(),
        edit_revision: 1,
        check_id: "check/price-presentation".into(),
        input_candidate: ComputeCandidateRefV2 {
            candidate_ref: "candidate/price-presentation".into(),
            candidate_revision: 1,
        },
        existing_source_id: None,
        expected_source_revision: None,
    };
    let draft = runtime.adapter.trusted_registered_draft(&request).unwrap();
    assert!(
        draft.inference_model_id.is_none(),
        "only the synthetic directory may be consulted; no inference"
    );
    runtime
        .register_manual_protected_input(
            request.input_candidate.clone(),
            ProtectedSecret::new(b"isolated-test-key".to_vec()).unwrap(),
        )
        .unwrap();
    let checked = runtime
        .adapter
        .check_registered_model_connection(request.clone())
        .unwrap();
    let candidate = checked
        .candidate
        .models
        .iter()
        .find(|model| model.upstream_model_id == model_id)
        .unwrap();
    assert!(candidate.selectable);
    let snapshot = runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .compute_management_snapshot(&WorkspaceId::default())
        .unwrap();
    let preview = runtime
        .adapter
        .preview_compute_save(ComputeManagementChangeV2 {
            schema: COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2.into(),
            subject: ComputeManagementSubjectV2::Candidate {
                candidate: checked.candidate.candidate.clone(),
            },
            expected_revisions: snapshot.revisions,
            selected_model_refs: vec![candidate.model_ref.clone()],
            intent: ComputeManagementIntentV2::SaveReady,
            key_edits: Vec::new(),
            validation: None,
            edit: None,
        })
        .unwrap();
    let service = crate::control::LocalControlDaemon::new(ApplicationService::new(
        runtime.application_ports(),
    ));
    let saved = service.dispatch_wire(LocalControlWireRequestV2 {
        schema_version: LOCAL_CONTROL_SCHEMA_V2,
        request_id: "price-presentation-save".into(),
        operation_id: "ApplyComputeConnectionChange".into(),
        payload: serde_json::to_value(ComputeConnectionApplyRequestV1 {
            spec: preview.spec,
            accept_digest: preview.accept_digest,
            expected_revisions: preview.expected_revisions,
            idempotency_key: "price-presentation-save".into(),
        })
        .unwrap(),
        protected_grant: None,
    });
    assert!(saved.error.is_none(), "{:?}", saved.error);
    assert_eq!(saved.operation.unwrap().state, "succeeded");
    runtime
        .release_manual_protected_input(&request.input_candidate.candidate_ref)
        .unwrap();

    let binding_id = runtime
        .adapter
        .compute_management_snapshot_v3(&ComputeManagementQueryV2::default())
        .unwrap()
        .sources[0]
        .models[0]
        .binding_id
        .clone();
    (root, runtime, binding_id)
}

#[test]
fn explicit_unpriced_paid_target_is_presented_and_remains_editable() {
    const TEST: &str = "control::runtime::model_connections::presentation::price_tests::explicit_unpriced_paid_target_is_presented_and_remains_editable";
    if crate::test_support::isolated_agent_home(TEST) {
        return;
    }
    let (_root, runtime, _) = saved_api("zhipu.general.cn.v1", "fixture-model");
    let management = || {
        runtime
            .adapter
            .compute_management_snapshot_v3(&ComputeManagementQueryV2::default())
            .unwrap()
    };
    let initial = management();
    assert_eq!(initial.sources.len(), 1);
    let model = &initial.sources[0].models[0];
    assert_eq!(model.presentation.billing_class, BillingClass::Paid);
    let binding_id = model.binding_id.clone();
    let target_context = ComputePriceContextV1 {
        currency: "USD".into(),
        valuation_kind: PriceValuationKindV1::ApiEquivalent,
    };
    assert!(
        !model.presentation.price_contexts.contains(&target_context),
        "an unbacked generated currency is not an editable context"
    );
    let prior_contexts = model.presentation.price_contexts.clone();
    let app = ApplicationService::new(runtime.application_ports());
    let initial_quote = effective(&app, &binding_id);
    assert!(matches!(
        initial_quote.items[0].quote.rates.input_uncached,
        TokenRateV1::Unknown { .. }
    ));
    assert!(matches!(
        initial_quote.items[0].quote.rates.output,
        TokenRateV1::Unknown { .. }
    ));
    for revision in 0..6 {
        assert_eq!(
            effective(&app, &binding_id).items[0]
                .edit_context
                .as_ref()
                .unwrap()
                .expected_override_revision,
            revision
        );
        apply_setting(
            &app,
            &binding_id,
            SourcePriceSettingV1::FollowCatalog,
            &format!("follow-catalog-{revision}"),
        );
    }
    let unknown = effective(&app, &binding_id);
    assert_eq!(
        unknown.items[0]
            .edit_context
            .as_ref()
            .unwrap()
            .expected_override_revision,
        6
    );
    assert!(matches!(
        unknown.items[0].quote.rates.input_uncached,
        TokenRateV1::Unknown { .. }
    ));
    assert!(matches!(
        unknown.items[0].quote.rates.output,
        TokenRateV1::Unknown { .. }
    ));
    let price_facts = runtime.adapter.price_control_facts().unwrap();
    let entry = price_facts
        .entries
        .iter()
        .find(|entry| {
            entry.execution_binding_id.as_deref() == Some(binding_id.as_str())
                && entry.target.currency == "USD"
                && entry.target.valuation_kind == PriceValuationKindV1::ApiEquivalent
        })
        .unwrap();
    let persisted = entry.source_override.as_ref().unwrap();
    assert_eq!(persisted.revision, 6);
    assert_eq!(persisted.setting, SourcePriceSettingV1::FollowCatalog);
    let presented = management();
    let presentation = &presented.sources[0].models[0].presentation;
    assert_eq!(presentation.billing_class, BillingClass::Paid);
    assert!(presentation.price_contexts.contains(&target_context));
    assert_eq!(presentation.price_contexts.len(), prior_contexts.len() + 1);
    assert!(
        prior_contexts
            .iter()
            .all(|context| presentation.price_contexts.contains(context))
    );

    let cache_unknown = TokenRateV1::unknown(PriceUnknownReasonV1::CacheRateNotCollected);
    apply_setting(
        &app,
        &binding_id,
        SourcePriceSettingV1::Set {
            rates: TokenRatesV1 {
                input_uncached: TokenRateV1::known(800_000),
                output: TokenRateV1::known(2_000_000),
                cache_read: cache_unknown.clone(),
                cache_write: cache_unknown.clone(),
            },
        },
        "manual-price",
    );
    let after = effective(&app, &binding_id);
    assert_eq!(
        after.items[0]
            .edit_context
            .as_ref()
            .unwrap()
            .expected_override_revision,
        7
    );
    assert_eq!(after.items[0].quote.origin, PriceOriginV1::Manual);
    assert_eq!(
        after.items[0].quote.rates.input_uncached,
        TokenRateV1::known(800_000)
    );
    assert_eq!(
        after.items[0].quote.rates.output,
        TokenRateV1::known(2_000_000)
    );
    assert_eq!(after.items[0].quote.rates.cache_read, cache_unknown);
    assert!(matches!(
        after.items[0].quote.rates.cache_write,
        TokenRateV1::Unknown { .. }
    ));
    let missing = app.dispatch(LocalControlRequestV2 {
        schema_version: LOCAL_CONTROL_SCHEMA_V2, request_id: "price-wrong-binding".into(),
        principal: PrincipalV1::interactive_user(), operation_id: "GetEffectivePrices".into(),
        payload: json!({"targets":[{"query_id":"wrong","target_locator":{"kind":"binding","binding_id":"binding/not-present"},"currency":"USD","valuation_kind":"api_equivalent"}]}),
        protected_grant: None,
    });
    assert_eq!(missing.error.unwrap().code, ErrorCode::ResourceNotFound);
}

#[test]
fn fresh_registered_api_prices_are_editable_in_the_key_budget_currency() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::model_connections::presentation::price_tests::fresh_registered_api_prices_are_editable_in_the_key_budget_currency",
    ) {
        return;
    }
    for (option, model_id) in [
        ("openai.platform.global.v1", "gpt-6.1-sol"),
        ("anthropic.platform.global.v1", "claude-opus-5-5"),
    ] {
        let (_root, runtime, binding_id) = saved_api(option, model_id);
        let app = ApplicationService::new(runtime.application_ports());
        let management = || {
            runtime
                .adapter
                .compute_management_snapshot_v3(&ComputeManagementQueryV2::default())
                .unwrap()
        };
        let initial = management();
        let source = &initial.sources[0];
        assert_eq!(
            source.models[0].presentation.billing_class,
            BillingClass::Paid
        );
        let context = |currency: &str| ComputePriceContextV1 {
            currency: currency.into(),
            valuation_kind: PriceValuationKindV1::UsageEstimate,
        };
        assert!(
            source.models[0]
                .presentation
                .price_contexts
                .contains(&context("USD"))
        );
        let quote = |currency: &str| -> EffectivePricesResultV2 {
            dispatch(
                &app,
                "GetEffectivePrices",
                json!({"targets":[{"query_id":"budget","target_locator":{"kind":"binding","binding_id":binding_id},"currency":currency,"valuation_kind":"usage_estimate"}]}),
            )
        };
        let unknown = quote("USD");
        assert!(matches!(
            unknown.items[0].quote.rates.input_uncached,
            TokenRateV1::Unknown { .. }
        ));
        assert_eq!(
            unknown.items[0]
                .edit_context
                .as_ref()
                .unwrap()
                .expected_override_revision,
            0
        );

        let preview: ComputeSavePreviewV2 = dispatch(
            &app,
            "PreviewComputeKeyBudgets",
            json!({
                "change":{"schema":"hiroute.compute-management-change/v3","change":{
                    "schema":"hiroute.compute-management-change/v2","subject":{"kind":"saved_source","source_id":source.source_id},
                    "expected_revisions":initial.revisions,"selected_model_refs":[source.models[0].model_ref],"intent":"save_ready","key_edits":[]}},
                "network_edit":null,"key_budget_edits":[{"key":{"kind":"saved","key_id":source.keys[0].key_id},"assignment":{
                    "intent":"configure","budget_id":null,"expected_revision":0,"display_name":"CNY allowance",
                    "policy":{"currency":"CNY","amount_micros":1000000,"renewal":null},"initial_remaining_micros":null}}]
            }),
        );
        let result: serde_json::Value = dispatch(
            &app,
            "ApplyComputeConnectionChange",
            ComputeConnectionApplyRequestV1 {
                spec: preview.spec,
                accept_digest: preview.accept_digest,
                expected_revisions: preview.expected_revisions,
                idempotency_key: "budget-price-context".into(),
            },
        );
        assert_eq!(result["state"], "succeeded");
        assert!(
            management().sources[0].models[0]
                .presentation
                .price_contexts
                .contains(&context("CNY"))
        );
        let before = quote("CNY");
        let edit = before.items[0].edit_context.as_ref().unwrap();
        assert_eq!(edit.expected_override_revision, 0);
        let preview: PriceOverridePreviewV2 = dispatch(
            &app,
            "PreviewPriceOverrideChange",
            json!({
                "target_locator":edit.target_locator,"expected_source_revision":edit.expected_source_revision,
                "expected_binding_revision":edit.expected_binding_revision,"expected_override_revision":0,
                "currency":"CNY","valuation_kind":"usage_estimate","action":{"kind":"set","rates":{
                    "input_uncached":{"state":"known","micros_per_million_tokens":1000000},"output":{"state":"known","micros_per_million_tokens":2000000},
                    "cache_read":{"state":"known","micros_per_million_tokens":0},"cache_write":{"state":"known","micros_per_million_tokens":1250000}}}
            }),
        );
        let result: serde_json::Value = dispatch(
            &app,
            "ApplyPriceOverrideChange",
            ApplyPriceOverrideChangeV2 {
                spec: preview.spec,
                accept_digest: preview.change_digest,
                expected_revisions: preview.expected_revisions,
                idempotency_key: "complete-budget-price".into(),
            },
        );
        assert_eq!(result["state"], "succeeded");
        let after = quote("CNY");
        assert_eq!(after.items[0].quote.origin, PriceOriginV1::Manual);
        assert_eq!(after.items[0].quote.rates.cache_read, TokenRateV1::known(0));
        assert_eq!(
            after.items[0].quote.rates.cache_write,
            TokenRateV1::known(1_250_000)
        );
        assert!(after.items[0].quote.unknown_reasons.is_empty());
    }
}
