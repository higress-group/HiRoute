use super::*;

#[test]
fn registered_model_defaults_keep_glm_custom_subsets_and_native_companions() {
    let catalog = crate::release_catalog::current_fixture_catalog();
    let mut fact = registered_fact(
        &catalog,
        "zhipu.coding-chat.cn.v1",
        "model.zhipu.glm-5.3.coding-chat",
    );
    let custom = NativeReasoningCapabilityV1::Discrete {
        parameter: "reasoning_effort".into(),
        profiles: vec!["low".into(), "high".into()],
        default_profile: Some("low".into()),
    };
    fact.default_preferences.insert(
        hiroute_domain::ModelDefaultFieldV1::NativeReasoning,
        hiroute_domain::ModelDefaultPreferenceV1::Custom {
            value: hiroute_domain::ModelDefaultValueV1::Reasoning(custom.clone()),
        },
    );
    let candidate = materialize_registered_management_candidate(&fact, &catalog)
        .expect("a custom subset must remain routable");
    assert_eq!(candidate.reasoning, custom);
    let profiles = &candidate.protocol_profiles[0].reasoning_profiles;
    assert_eq!(
        profiles
            .iter()
            .map(|p| p.profile_id.as_str())
            .collect::<Vec<_>>(),
        ["low", "high"]
    );
    for profile in profiles {
        assert_eq!(
            profile.render,
            GatewayNativeReasoningRenderV1::ExactFields {
                protocol: UpstreamProtocol::ChatCompletions,
                fields: vec![
                    field(
                        "reasoning_effort",
                        GatewayNativeReasoningValueV1::String(profile.profile_id.clone()),
                        UpstreamProtocol::ChatCompletions
                    ),
                    field(
                        "thinking.type",
                        GatewayNativeReasoningValueV1::String("enabled".into()),
                        UpstreamProtocol::ChatCompletions
                    ),
                    field(
                        "thinking.clear_thinking",
                        GatewayNativeReasoningValueV1::Bool(false),
                        UpstreamProtocol::ChatCompletions
                    )
                ],
            }
        );
    }
    // Unsupported native names and wrong protocols still fail; custom settings do not relax the wire contract.
    let mut native = catalog
        .native_reasoning()
        .iter()
        .find(|n| n.model_configuration_id == "model.zhipu.glm-5.3.coding-chat")
        .unwrap()
        .clone();
    native = native.with_source_capability(custom);
    assert!(
        native
            .validate_for_protocol(UpstreamProtocol::Messages)
            .is_err()
    );
    if let NativeReasoningCapabilityV1::Discrete { profiles, .. } = &mut native.capability {
        profiles.push("invented".into());
    }
    assert!(native.validate().is_err());
}

#[test]
fn registered_model_defaults_follow_new_data_without_changing_authenticated_saved_capabilities() {
    let installed = crate::release_catalog::current_fixture_catalog();
    let id = "model.bailian.qwen3-max-2026-01-23";
    let original = registered_fact(&installed, "bailian.payg.cn.v1", id);
    let before = original.capabilities.clone();
    let evidence = original.capability_evidence_digest.clone();
    let mut snapshot = installed.official_snapshot();
    let model = snapshot
        .data
        .models
        .iter_mut()
        .find(|m| m.model_configuration_id == id)
        .unwrap();
    let old_context = model.capabilities.context_tokens;
    model.capabilities.context_tokens += 1234;
    let current_context = model.capabilities.context_tokens;
    snapshot.data.rating_snapshot.model_catalog_digest =
        CanonicalDigest::of(&snapshot.data.models).unwrap();
    snapshot.data.rating_snapshot.digest = snapshot.data.rating_snapshot.computed_digest().unwrap();
    snapshot.data_revision += 1;
    snapshot.data_digest = CanonicalDigest::of(&snapshot.data).unwrap();
    let payload = hiroute_integrations::model_feed::FeedPayload::from_snapshot(
        snapshot,
        42,
        "2026-10-18T00:00:00Z".into(),
        "1".repeat(40),
        vec![],
    );
    let admission = hiroute_integrations::model_feed::admit(&installed, &payload, "0.3.2").unwrap();
    let current = installed.with_feed(&admission).unwrap();
    let resolved = materialize_registered_management_candidate(&original, &current).unwrap();
    assert_eq!(resolved.model.capabilities.context_tokens, current_context);
    assert_eq!(original.capabilities, before);
    assert_eq!(original.capability_evidence_digest, evidence);
    let mut preserved = original.clone();
    preserved.default_preferences.insert(
        hiroute_domain::ModelDefaultFieldV1::ContextTokens,
        hiroute_domain::ModelDefaultPreferenceV1::PreserveSaved,
    );
    assert_eq!(
        materialize_registered_management_candidate(&preserved, &current)
            .unwrap()
            .model
            .capabilities
            .context_tokens,
        old_context
    );
    let mut tampered = original;
    tampered.capabilities.context_tokens.value = Some(123456);
    assert!(materialize_registered_management_candidate(&tampered, &current).is_none());
}
