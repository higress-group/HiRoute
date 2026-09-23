use std::collections::BTreeMap;

use hiroute_domain::*;
use hiroute_integrations::TrustedReleaseCatalog;

fn assert_no_numeric_cost_leaf(value: &serde_json::Value) {
    match value {
        serde_json::Value::Number(number) => {
            panic!("cost hint contains executable numeric leaf: {number}")
        }
        serde_json::Value::Array(values) => values.iter().for_each(assert_no_numeric_cost_leaf),
        serde_json::Value::Object(values) => values.values().for_each(assert_no_numeric_cost_leaf),
        _ => {}
    }
}

fn fixture() -> (ConnectorRegistryBundleV1, ReleaseModelDataBundleV2) {
    (
        serde_json::from_slice(include_bytes!(
            "../../../assets/release-facts/current/bundle/connector-registry.json"
        ))
        .unwrap(),
        serde_json::from_slice(include_bytes!(
            "../../../assets/release-facts/current/bundle/model-data.json"
        ))
        .unwrap(),
    )
}
fn encoded(
    registry: &ConnectorRegistryBundleV1,
    data: &ReleaseModelDataBundleV2,
) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let r = serde_json::to_vec(registry).unwrap();
    let d = serde_json::to_vec(data).unwrap();
    let manifest = ReleaseFactsManifestV2 {
        schema: RELEASE_FACTS_SCHEMA_V2.into(),
        tool_version: RELEASE_FACTS_TOOL_VERSION_V2.into(),
        catalog_id: "fixture/current".into(),
        product_release: data.data.product_release.clone(),
        sequence: 1,
        connector_registry_digest: CanonicalDigest::of_bytes(&r),
        model_data_digest: CanonicalDigest::of_bytes(&d),
        cross_reference_digest: data.cross_reference_digest(registry).unwrap(),
    };
    (serde_json::to_vec(&manifest).unwrap(), r, d)
}
#[test]
fn current_model_metadata_loader_accepts_sparse_scores_and_rejects_tampering() {
    let (r, mut d) = fixture();
    d.rating_snapshot.records.clear();
    d.rating_snapshot.digest = d.rating_snapshot.computed_digest().unwrap();
    let (m, r, mut bytes) = encoded(&r, &d);
    let catalog = TrustedReleaseCatalog::load_release_facts(&m, &r, &bytes).unwrap();
    assert!(catalog.model_data().ratings.is_empty());
    assert!(catalog.rating_snapshot().records.is_empty());
    assert!(!catalog.native_reasoning().is_empty());
    bytes.push(b' ');
    assert!(TrustedReleaseCatalog::load_release_facts(&m, &r, &bytes).is_err());
    let mut manifest: ReleaseFactsManifestV2 = serde_json::from_slice(&m).unwrap();
    manifest.catalog_id.clear();
    assert!(
        TrustedReleaseCatalog::load_release_facts(
            &serde_json::to_vec(&manifest).unwrap(),
            &r,
            &bytes[..bytes.len() - 1]
        )
        .is_err()
    );
}
#[test]
fn current_model_metadata_rejects_mixed_old_scores_and_wrong_model_digest() {
    let (r, mut d) = fixture();
    d.data.ratings.push(RatingV1 {
        model_configuration_id: d.data.models[0].model_configuration_id.clone(),
        overall_score_tenths: 45,
        rating_count: 1,
    });
    assert!(d.validate_against(&r).is_err());
    d.data.ratings.clear();
    d.rating_snapshot.model_catalog_digest = CanonicalDigest::of_bytes(b"wrong");
    d.rating_snapshot.digest = d.rating_snapshot.computed_digest().unwrap();
    assert!(d.validate_against(&r).is_err());
}

#[test]
fn current_model_metadata_has_valid_digest_and_no_score_spreading() {
    let registry: ConnectorRegistryBundleV1 = serde_json::from_slice(include_bytes!(
        "../../../assets/release-facts/current/bundle/connector-registry.json"
    ))
    .unwrap();
    let data: ReleaseModelDataBundleV2 = serde_json::from_slice(include_bytes!(
        "../../../assets/release-facts/current/bundle/model-data.json"
    ))
    .unwrap();
    data.validate_against(&registry).unwrap();
    assert_eq!(data.data.models.len(), 19);
    assert_eq!(data.rating_snapshot.records.len(), 46);
    assert!(
        data.data
            .model_endpoint_capabilities
            .iter()
            .all(|capability| !capability.capability_id.starts_with("cap.runtime.")),
        "incomplete bindings must not become executable capabilities"
    );
    assert_eq!(data.metadata_catalog.provider_records.len(), 105);
    assert_eq!(data.metadata_catalog.model_records.len(), 764);
    assert_eq!(data.metadata_catalog.inference_rules.len(), 187);
    for (model_key, provider_key) in [
        ("claude-opus-5-5", "official/anthropic-api"),
        ("gpt-6-luna", "official/openai-platform"),
        ("gpt-6-sol", "official/openai-platform"),
    ] {
        assert!(
            data.metadata_catalog
                .canonical_models
                .iter()
                .any(|model| model.model_key == model_key)
        );
        let record = data
            .metadata_catalog
            .model_records
            .iter()
            .find(|record| {
                record.provider_record_key == provider_key && record.upstream_model_id == model_key
            })
            .unwrap();
        assert!(
            record
                .usable_for
                .contains(&MetadataUsageScenarioV1::CustomApiModelPrefill)
        );
        let binding = data
            .metadata_catalog
            .endpoint_bindings
            .iter()
            .find(|binding| binding.model_key == model_key)
            .unwrap();
        assert_eq!(binding.availability.state, "conditional");
        assert_eq!(binding.protocol_qualification, "runtime-required");
        assert!(
            data.data
                .models
                .iter()
                .all(|model| !model.model_configuration_id.ends_with(model_key))
        );
    }
    // The dataset is a closed, determinate snapshot: no provider-scoped record may keep an
    // unknown capability, limit, lifecycle, rendering, or cost-hint outcome, and every
    // rule-closed field must name a rule that the same client-bundled catalog carries.
    let rules: Vec<&str> = data
        .metadata_catalog
        .inference_rules
        .iter()
        .map(|rule| rule.rule_key.as_str())
        .collect();
    for record in &data.metadata_catalog.model_records {
        assert_ne!(record.context_tokens.state, MetadataTokenStateV1::Unknown);
        assert_ne!(
            record.max_output_tokens.state,
            MetadataTokenStateV1::Unknown
        );
        assert_ne!(record.lifecycle, "unknown");
        assert!(!record.input_modalities.is_empty());
        for state in [
            record.capability_hints.reasoning,
            record.capability_hints.streaming,
            record.capability_hints.tool,
            record.capability_hints.vision,
        ] {
            assert_ne!(state, MetadataCapabilityStateV1::Unknown);
        }
        let rendering = &record.reasoning_rendering_hints;
        let has_hints = !rendering.reasoning_effort_maps.is_empty()
            || !rendering.supported_reasoning_efforts.is_empty()
            || !rendering.thinking_level_maps.is_empty();
        assert_eq!(
            has_hints,
            record.reasoning_rendering_state.is_none(),
            "{}",
            record.model_record_key
        );
        for provenance in record.field_provenance.values() {
            assert!(rules.contains(&provenance.rule_key.as_str()));
        }
    }
    for record in &data.metadata_catalog.provider_records {
        for provenance in record.field_provenance.values() {
            assert!(rules.contains(&provenance.rule_key.as_str()));
        }
    }
    let inferred = data
        .metadata_catalog
        .model_records
        .iter()
        .find(|record| record.model_record_key == "hermes-agent/ai-gateway/google%2Fgemini-3-flash")
        .unwrap();
    assert_eq!(
        inferred.context_tokens,
        MetadataTokenLimitV1 {
            state: MetadataTokenStateV1::Known,
            value: Some(1_048_576),
            candidates: Vec::new(),
        }
    );
    assert_eq!(
        inferred.capability_hints.vision,
        MetadataCapabilityStateV1::Supported
    );
    assert_eq!(
        inferred.field_provenance["capability_hints.streaming"].basis,
        MetadataInferenceBasisV1::Inferred
    );
    let unsupported = data
        .metadata_catalog
        .model_records
        .iter()
        .find(|record| record.model_record_key == "openclaw/baseten/deepseek-ai%2FDeepSeek-V4-Pro")
        .unwrap();
    assert_eq!(
        unsupported.capability_hints.vision,
        MetadataCapabilityStateV1::Unsupported
    );
    let image_only = data
        .metadata_catalog
        .model_records
        .iter()
        .find(|record| record.model_record_key == "hermes-agent/openai-codex/gpt-image-2")
        .unwrap();
    assert_eq!(
        image_only.execution_fit.state,
        MetadataExecutionFitStateV1::NotApplicable
    );
    assert_eq!(
        image_only.context_tokens.state,
        MetadataTokenStateV1::NotApplicable
    );
    let reviewer = data
        .metadata_catalog
        .model_records
        .iter()
        .find(|record| record.model_record_key == "hermes-agent/openai-codex/codex-auto-review")
        .unwrap();
    assert_eq!(
        reviewer.execution_fit.state,
        MetadataExecutionFitStateV1::Unsupported
    );
    assert_eq!(
        reviewer.cost_hint_state,
        MetadataCostHintStateV1::NotRecorded
    );
    let tiered = data
        .metadata_catalog
        .model_records
        .iter()
        .find(|record| record.model_record_key == "openclaw/openai/gpt-6-astra")
        .unwrap();
    assert!(tiered.cost_hints[0]["tieredPricing"].is_array());
    assert_eq!(tiered.cost_hint_state, MetadataCostHintStateV1::Recorded);
    data.metadata_catalog
        .model_records
        .iter()
        .for_each(|record| {
            record
                .cost_hints
                .iter()
                .flat_map(|hint| hint.values())
                .for_each(assert_no_numeric_cost_leaf);
        });

    let mut invalid_binding = data.clone();
    invalid_binding.metadata_catalog.endpoint_bindings[0].interface_candidates =
        vec!["anthropic-platform/anthropic-messages".into()];
    assert_eq!(
        invalid_binding.validate_against(&registry),
        Err(ComputeContractError::CrossReference)
    );

    let mut numeric_cost = data.clone();
    numeric_cost
        .metadata_catalog
        .model_records
        .iter_mut()
        .find(|record| !record.cost_hints.is_empty())
        .unwrap()
        .cost_hints[0]
        .insert("forged_numeric_price".into(), serde_json::json!(1.25));
    assert!(numeric_cost.validate_against(&registry).is_err());

    let bailian_model = "model.bailian.qwen3-max-2026-01-23";
    let bailian_capability = data
        .data
        .model_endpoint_capabilities
        .iter()
        .find(|capability| capability.model_configuration_id == bailian_model)
        .unwrap();
    assert_eq!(
        bailian_capability.endpoint_profile_id,
        "endpoint.bailian.payg.cn.v1"
    );
    assert_eq!(
        bailian_capability.upstream_protocol,
        UpstreamProtocol::ChatCompletions
    );
    assert_eq!(bailian_capability.upstream_model_id, "qwen3-max-2026-01-23");
    let bailian_offer = data
        .data
        .offers
        .iter()
        .find(|offer| {
            offer
                .model_configuration_ids
                .iter()
                .any(|id| id == bailian_model)
        })
        .unwrap();
    assert_eq!(bailian_offer.billing_class, BillingClass::Paid);
    assert_eq!(bailian_offer.region_id, "cn");
    assert!(
        data.data
            .price_rates
            .iter()
            .all(|rate| { rate.model_configuration_id != bailian_model })
    );
    assert!(data.rating_snapshot.models.iter().any(|native| {
        native.model_configuration_id == bailian_model
            && native.capability
                == NativeReasoningCapabilityV1::Toggle {
                    parameter: "enable_thinking".into(),
                }
    }));
    let spark: Vec<_> = data
        .rating_snapshot
        .records
        .iter()
        .filter(|r| r.model_configuration_id == "model.openai.gpt-5.3-codex-spark")
        .collect();
    assert_eq!(spark.len(), 1);
    assert_eq!(
        spark[0].native_configuration,
        NativeRatingConfigurationV1::Profile {
            profile: "xhigh".into()
        }
    );
    assert!(matches!(
        spark[0].overall,
        RatingValueV1::Estimated {
            score_tenths: 45,
            ..
        }
    ));
    assert!(data.rating_snapshot.records.iter().all(|r| matches!(
        r.coding,
        RatingValueV1::Unknown { .. }
    ) && matches!(
        r.tool,
        RatingValueV1::Unknown { .. }
    )));
    let (manifest, registry, bytes) = encoded(&registry, &data);
    TrustedReleaseCatalog::load_release_facts(&manifest, &registry, &bytes).unwrap();
}

#[test]
fn codex_subscription_inventory_resolves_only_the_promoted_models() {
    let registry: ConnectorRegistryBundleV1 = serde_json::from_slice(include_bytes!(
        "../../../assets/release-facts/current/bundle/connector-registry.json"
    ))
    .unwrap();
    let data: ReleaseModelDataBundleV2 = serde_json::from_slice(include_bytes!(
        "../../../assets/release-facts/current/bundle/model-data.json"
    ))
    .unwrap();
    data.validate_against(&registry).unwrap();
    let observed = [
        "codex-auto-review",
        "gpt-5.3-codex-spark",
        "gpt-5.5",
        "gpt-5.6-luna",
        "gpt-5.6-sol",
        "gpt-5.6-terra",
        "gpt-6-astra",
        "gpt-image-1.5",
        "gpt-image-2",
    ]
    .map(|upstream_model_id| ObservedModelV1 {
        upstream_model_id: upstream_model_id.into(),
        metadata: BTreeMap::new(),
    });
    let resolved = reconcile_inventory("endpoint.cpa.codex", observed, &data.data).unwrap();
    let matched: Vec<_> = resolved
        .iter()
        .filter(|model| model.disposition == InventoryDisposition::CatalogMatched)
        .map(|model| model.upstream_model_id.as_str())
        .collect();
    assert_eq!(
        matched,
        [
            "gpt-5.3-codex-spark",
            "gpt-5.5",
            "gpt-5.6-luna",
            "gpt-5.6-sol",
            "gpt-5.6-terra",
            "gpt-6-astra",
        ]
    );
    let offer = data
        .data
        .offers
        .iter()
        .find(|offer| offer.offer_id == "offer.codex.subscription")
        .unwrap();
    for model in resolved
        .iter()
        .filter(|model| model.disposition == InventoryDisposition::CatalogMatched)
    {
        let configuration_id = model.model_configuration_id.as_deref().unwrap();
        assert!(
            offer
                .model_configuration_ids
                .iter()
                .any(|id| id == configuration_id),
            "{configuration_id} is catalog-matched but not offered on the subscription"
        );
        assert!(
            data.rating_snapshot
                .models
                .iter()
                .any(|native| native.model_configuration_id == configuration_id),
            "{configuration_id} has no native reasoning record"
        );
    }
    // The image models and the internal reviewer entry keep complete metadata but must never
    // resolve to a text capability on the subscription endpoint.
    for excerpt in ["gpt-image-1.5", "gpt-image-2", "codex-auto-review"] {
        let entry = resolved
            .iter()
            .find(|model| model.upstream_model_id == excerpt)
            .unwrap();
        assert_eq!(entry.disposition, InventoryDisposition::InventoryOnly);
        assert_eq!(entry.model_configuration_id, None);
    }
}

#[test]
fn claude_effort_convention_is_exposed_without_a_second_rating_axis() {
    let data: ReleaseModelDataBundleV2 = serde_json::from_slice(include_bytes!(
        "../../../assets/model-data/current/model-data.json"
    ))
    .unwrap();
    for id in [
        "model.anthropic.claude-opus-5",
        "model.anthropic.claude-sonnet-5",
    ] {
        let native = data
            .rating_snapshot
            .models
            .iter()
            .find(|m| m.model_configuration_id == id)
            .unwrap();
        assert_eq!(
            native.native_render_convention,
            Some(NativeReasoningRenderConventionV1::ClaudeAdaptiveEffortMessages)
        );
        native
            .validate_for_protocol(UpstreamProtocol::Messages)
            .unwrap();
        assert!(
            native
                .validate_for_protocol(UpstreamProtocol::Responses)
                .is_err()
        );
        assert!(
            native
                .validate_for_protocol(UpstreamProtocol::ChatCompletions)
                .is_err()
        );
        let view =
            model_reference_view(&data.data, &data.rating_snapshot.models, vec![], id).unwrap();
        assert_eq!(
            view.native_render_convention,
            native.native_render_convention
        );
        for profile in ["low", "medium", "high", "xhigh", "max"] {
            let resolved = data
                .rating_snapshot
                .resolve(
                    id,
                    &ExactNativeReasoningV1::Profile {
                        parameter: "output_config.effort".into(),
                        profile: profile.into(),
                        render_mode: ReasoningRenderModeV1::ExplicitNative,
                    },
                )
                .unwrap();
            assert!(matches!(
                resolved.overall,
                RatingValueV1::Unknown {
                    reason: RatingUnknownReasonV1::RatingNotCollected
                }
            ));
            assert!(matches!(resolved.coding, RatingValueV1::Unknown { .. }));
            assert!(matches!(resolved.tool, RatingValueV1::Unknown { .. }));
        }
        let mut altered = data.rating_snapshot.clone();
        altered
            .models
            .iter_mut()
            .find(|m| m.model_configuration_id == id)
            .unwrap()
            .native_render_convention = None;
        assert_ne!(
            altered.computed_digest().unwrap(),
            data.rating_snapshot.digest
        );
        assert!(altered.validate().is_err());
        for capability in [
            NativeReasoningCapabilityV1::Fixed {
                profile: "fixed".into(),
            },
            NativeReasoningCapabilityV1::Toggle {
                parameter: "thinking.type".into(),
            },
            NativeReasoningCapabilityV1::Budget {
                parameter: "thinking.budget_tokens".into(),
                minimum_tokens: 1024,
                maximum_tokens: 8192,
                step_tokens: 1024,
            },
            NativeReasoningCapabilityV1::Discrete {
                parameter: "reasoning.effort".into(),
                profiles: vec!["high".into()],
            },
        ] {
            let mut invalid = native.clone();
            invalid.capability = capability;
            assert!(invalid.validate().is_err());
        }
        let mut unknown = serde_json::to_value(native).unwrap();
        unknown["native_render_convention"] = "unrecognized".into();
        assert!(serde_json::from_value::<ModelNativeReasoningV1>(unknown).is_err());
    }
    let old: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../assets/model-data/current/rating-seed.json"
    ))
    .unwrap();
    let old_native: Vec<ModelNativeReasoningV1> =
        serde_json::from_value(old["native_reasoning"].clone()).unwrap();
    assert!(
        old_native
            .iter()
            .all(|m| m.native_render_convention.is_none())
    );
    assert_eq!(
        serde_json::to_value(old_native).unwrap(),
        old["native_reasoning"]
    );
}

#[test]
fn claude_effort_convention_is_preserved_by_client_bundled_catalog() {
    let data: ReleaseModelDataBundleV2 = serde_json::from_slice(include_bytes!(
        "../../../assets/release-facts/current/bundle/model-data.json"
    ))
    .unwrap();
    let registry: ConnectorRegistryBundleV1 = serde_json::from_slice(include_bytes!(
        "../../../assets/release-facts/current/bundle/connector-registry.json"
    ))
    .unwrap();
    let (manifest, registry_bytes, data_bytes) = encoded(&registry, &data);
    let catalog =
        TrustedReleaseCatalog::load_release_facts(&manifest, &registry_bytes, &data_bytes).unwrap();
    assert_eq!(catalog.native_reasoning(), data.rating_snapshot.models);
    for id in [
        "model.anthropic.claude-opus-5",
        "model.anthropic.claude-sonnet-5",
    ] {
        let native = data
            .rating_snapshot
            .models
            .iter()
            .find(|m| m.model_configuration_id == id)
            .unwrap();
        assert_eq!(
            native.native_render_convention,
            Some(NativeReasoningRenderConventionV1::ClaudeAdaptiveEffortMessages)
        );
        native
            .validate_for_protocol(UpstreamProtocol::Messages)
            .unwrap();
        assert!(
            native
                .validate_for_protocol(UpstreamProtocol::Responses)
                .is_err()
        );
        assert!(
            native
                .validate_for_protocol(UpstreamProtocol::ChatCompletions)
                .is_err()
        );
        for profile in ["low", "medium", "high", "xhigh", "max"] {
            let resolved = data
                .rating_snapshot
                .resolve(
                    id,
                    &ExactNativeReasoningV1::Profile {
                        parameter: "output_config.effort".into(),
                        profile: profile.into(),
                        render_mode: ReasoningRenderModeV1::ExplicitNative,
                    },
                )
                .unwrap();
            assert!(matches!(
                resolved.overall,
                RatingValueV1::Unknown {
                    reason: RatingUnknownReasonV1::RatingNotCollected
                }
            ));
            assert!(matches!(resolved.coding, RatingValueV1::Unknown { .. }));
            assert!(matches!(resolved.tool, RatingValueV1::Unknown { .. }));
        }
        let mut altered = data.rating_snapshot.clone();
        altered
            .models
            .iter_mut()
            .find(|m| m.model_configuration_id == id)
            .unwrap()
            .native_render_convention = None;
        assert_ne!(
            altered.computed_digest().unwrap(),
            data.rating_snapshot.digest
        );
        assert!(altered.validate().is_err());
        for capability in [
            NativeReasoningCapabilityV1::Fixed {
                profile: "fixed".into(),
            },
            NativeReasoningCapabilityV1::Toggle {
                parameter: "thinking.type".into(),
            },
            NativeReasoningCapabilityV1::Budget {
                parameter: "thinking.budget_tokens".into(),
                minimum_tokens: 1024,
                maximum_tokens: 8192,
                step_tokens: 1024,
            },
            NativeReasoningCapabilityV1::Discrete {
                parameter: "reasoning.effort".into(),
                profiles: vec!["high".into()],
            },
        ] {
            let mut invalid = native.clone();
            invalid.capability = capability;
            assert!(invalid.validate().is_err());
        }
        let mut unknown = serde_json::to_value(native).unwrap();
        unknown["native_render_convention"] = "unrecognized".into();
        assert!(serde_json::from_value::<ModelNativeReasoningV1>(unknown).is_err());
    }
    let old: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../assets/model-data/current/rating-seed.json"
    ))
    .unwrap();
    let old_native: Vec<ModelNativeReasoningV1> =
        serde_json::from_value(old["native_reasoning"].clone()).unwrap();
    assert!(
        old_native
            .iter()
            .all(|m| m.native_render_convention.is_none())
    );
    assert_eq!(
        serde_json::to_value(old_native).unwrap(),
        old["native_reasoning"]
    );
}

#[test]
fn connection_templates_cover_current_api_and_free_options_without_inventory_gates() {
    let catalog = hiroute_integrations::TrustedReleaseCatalog::load_bundled_release_facts(
        include_bytes!("../../../assets/release-facts/current/bundle/manifest.json"),
        include_bytes!("../../../assets/release-facts/current/bundle/manifest.json"),
        include_bytes!("../../../assets/release-facts/current/bundle/connector-registry.json"),
        include_bytes!("../../../assets/release-facts/current/bundle/model-data.json"),
    )
    .unwrap();
    let free = catalog
        .registry()
        .connection_options
        .iter()
        .filter(|option| option.billing_class == BillingClass::Free)
        .collect::<Vec<_>>();
    assert_eq!(free.len(), 5);
    for option in free {
        let resolved = catalog
            .resolve_connection_option(&option.connection_option_id)
            .unwrap();
        assert!(
            resolved
                .endpoint_profile
                .protocol_endpoints
                .iter()
                .all(|endpoint| endpoint.authentication_semantics
                    == Some(GatewayAuthenticationSemanticsV1::Bearer))
        );
    }
    for id in [
        "bailian.coding-plan.cn.v1",
        "bailian.token-plan.cn.v1",
        "kimi.code.cn.v1",
        "zhipu.coding-plan.cn.v1",
        "zai.general.global.v1",
        "zai.coding-plan.global.v1",
    ] {
        let resolved = catalog.resolve_connection_option(id).unwrap();
        assert!(
            resolved
                .endpoint_profile
                .protocol_endpoints
                .iter()
                .any(|endpoint| endpoint.authentication_semantics.is_some())
        );
    }
}
