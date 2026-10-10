use super::*;

fn id_validation_draft(id: &str) -> NativeModelConnectionDraftV1 {
    NativeModelConnectionDraftV1 {
        display_template_id: None,
        inference_model_id: None,
        candidate_ref: None,
        lineage_ref: "lineage/native/id-validation".into(),
        trusted_lineage_digest: None,
        display_name: "中文接入名称".into(),
        existing_source_id: None,
        edit_revision: 1,
        check_id: "check/id-validation".into(),
        base_url: "http://127.0.0.1:8123/v1".into(),
        base_kind: ModelConnectionBaseKindV1::ApiRoot,
        request_path_override: None,
        inventory_path_override: None,
        protocol: UpstreamProtocol::Responses,
        protocol_profile_id: "profile/custom/responses".into(),
        protocol_profile_revision: 1,
        protocol_header_semantics: GatewayHeaderSemanticsV1 {
            content_type: "application/json".into(),
            required_headers: Vec::new(),
            forbidden_forward_headers: Vec::new(),
        },
        authentication: GatewayAuthenticationSemanticsV1::None,
        additional_native_endpoints: Vec::new(),
        provenance: NativeConnectionProvenanceInputV1::UserConfigured {
            configuration_revision: 1,
        },
        qualification: NativeConnectionQualificationV1 {
            free_access: None,
            evidence_ref: None,
        },
        runtime_fallback_denied_model_ids: Default::default(),
        models: vec![NativeModelDeclarationV1 {
            upstream_model_id: id.into(),
            display_name: "中文模型名称".into(),
            catalog_configuration_id: None,
            membership: ComputeModelMembershipV2::UserDeclared,
            capabilities: NativeModelCapabilityDeclarationV1::default(),
        }],
    }
}

#[test]
fn native_draft_uses_the_same_opaque_id_contract_for_models_inference_and_denials() {
    for id in [
        "内网模型".to_owned(),
        "Model A/@revision?实验#1".into(),
        "\u{feff}模型\u{feff}".into(),
        "\u{feff}".into(),
        "模型🧠".into(),
        "🧠".repeat(128),
        "x".repeat(512),
        "界".repeat(170) + "ab",
    ] {
        let mut draft = id_validation_draft(&id);
        validate_draft(&draft, &NativeModelConnectionCredentialV1::NotRequired).unwrap();
        draft.inference_model_id = Some(id.clone());
        validate_draft(&draft, &NativeModelConnectionCredentialV1::NotRequired).unwrap();
        draft.inference_model_id = None;
        draft.runtime_fallback_denied_model_ids.insert(id);
        validate_draft(&draft, &NativeModelConnectionCredentialV1::NotRequired).unwrap();
    }
}

#[test]
fn native_draft_rejects_invalid_provider_ids_before_any_probe() {
    for id in [
        " bad".to_owned(),
        "bad\u{a0}".into(),
        "bad\tmodel".into(),
        "bad\0model".into(),
        "bad\u{85}model".into(),
        "\u{85}bad".into(),
        "bad\u{85}".into(),
        "界".repeat(171),
        "🧠".repeat(129),
    ] {
        let invalid = id_validation_draft(&id);
        assert!(matches!(
            validate_draft(&invalid, &NativeModelConnectionCredentialV1::NotRequired),
            Err(NativeModelConnectionErrorV1::InvalidDraft)
        ));
        let mut denied = id_validation_draft("内网模型");
        denied.runtime_fallback_denied_model_ids.insert(id);
        assert!(validate_draft(&denied, &NativeModelConnectionCredentialV1::NotRequired).is_err());
    }
}

fn known<T>(value: T) -> NativeCandidateFactValueV1<T> {
    NativeCandidateFactValueV1 {
        value: Some(value),
        basis: NativeCandidateFactBasisV1::UserDeclared,
    }
}

fn complete_capabilities(
    reasoning: NativeReasoningCapabilityV1,
) -> NativeModelCapabilityDeclarationV1 {
    NativeModelCapabilityDeclarationV1 {
        tool: known(true),
        vision: known(false),
        streaming: known(true),
        context_tokens: known(32_768),
        max_output_tokens: known(4_096),
        native_reasoning: known(reasoning),
    }
}

fn fixed_reasoning() -> NativeReasoningCapabilityV1 {
    NativeReasoningCapabilityV1::Fixed {
        profile: "provider-default".into(),
    }
}

fn model(id: &str, capabilities: NativeModelCapabilityDeclarationV1) -> NativeModelDeclarationV1 {
    NativeModelDeclarationV1 {
        upstream_model_id: id.into(),
        display_name: id.into(),
        catalog_configuration_id: None,
        membership: ComputeModelMembershipV2::UserDeclared,
        capabilities,
    }
}

fn assert_unknown_preserved(capabilities: NativeModelCapabilityDeclarationV1) {
    assert!(model_is_selectable(&capabilities));
    let candidate = candidate_models(
        "candidate/native/incomplete",
        &[model("incomplete-model", capabilities)],
        &[],
        true,
        false,
        &Default::default(),
    )
    .unwrap()
    .pop()
    .unwrap();
    assert!(candidate.selectable);
    assert_eq!(candidate.reason, None);
}

#[test]
fn unknown_capabilities_allow_text_selection_without_inventing_facts() {
    let complete = complete_capabilities(fixed_reasoning());
    assert!(model_is_selectable(&complete));

    let mut missing_tool = complete.clone();
    missing_tool.tool = NativeCandidateFactValueV1::unknown();
    assert_unknown_preserved(missing_tool);

    let mut missing_vision = complete.clone();
    missing_vision.vision = NativeCandidateFactValueV1::unknown();
    assert_unknown_preserved(missing_vision);

    let mut missing_streaming = complete.clone();
    missing_streaming.streaming = NativeCandidateFactValueV1::unknown();
    assert_unknown_preserved(missing_streaming);

    let mut missing_reasoning = complete;
    missing_reasoning.native_reasoning = NativeCandidateFactValueV1::unknown();
    assert_unknown_preserved(missing_reasoning);
}

#[test]
fn explicit_false_capability_facts_are_complete() {
    let mut declaration = complete_capabilities(fixed_reasoning());
    declaration.tool = known(false);
    declaration.vision = known(false);
    declaration.streaming = known(false);
    assert!(model_is_selectable(&declaration));
}

#[test]
fn native_reasoning_shapes_are_preserved_without_effort_mapping() {
    let native = vec![
        NativeReasoningCapabilityV1::Fixed {
            profile: "provider-default".into(),
        },
        NativeReasoningCapabilityV1::Discrete {
            parameter: "output_config.effort".into(),
            profiles: vec!["low".into(), "max".into()],
            default_profile: None,
        },
        NativeReasoningCapabilityV1::Budget {
            parameter: "thinking.budget_tokens".into(),
            minimum_tokens: 1_024,
            maximum_tokens: 4_096,
            step_tokens: 1_024,
        },
    ];

    for reasoning in native {
        let declaration = complete_capabilities(reasoning.clone());
        assert!(model_is_selectable(&declaration));
        let converted = convert_capabilities(&declaration);
        assert_eq!(converted.native_reasoning.value, Some(reasoning));
        assert_eq!(
            converted.native_reasoning.basis,
            ComputeCandidateFactBasisV2::UserDeclared
        );
    }
}

#[test]
fn invalid_bounds_are_rejected_but_missing_limits_remain_unknown() {
    let complete = complete_capabilities(fixed_reasoning());

    let mut missing = complete.clone();
    missing.context_tokens = NativeCandidateFactValueV1::unknown();
    assert!(model_is_selectable(&missing));

    let mut inverted = complete;
    inverted.context_tokens = known(4_096);
    inverted.max_output_tokens = known(8_192);
    assert!(!model_is_selectable(&inverted));

    let mut invalid_reasoning = complete_capabilities(fixed_reasoning());
    invalid_reasoning.native_reasoning = known(NativeReasoningCapabilityV1::Discrete {
        parameter: "reasoning_effort".into(),
        profiles: Vec::new(),
        default_profile: None,
    });
    assert!(!model_is_selectable(&invalid_reasoning));
}

#[test]
fn incomplete_model_does_not_block_complete_model_selection() {
    let mut incomplete = complete_capabilities(fixed_reasoning());
    incomplete.vision = NativeCandidateFactValueV1::unknown();
    let models = candidate_models(
        "candidate/native/mixed",
        &[
            model("incomplete-model", incomplete),
            model("ready-model", complete_capabilities(fixed_reasoning())),
        ],
        &[],
        true,
        false,
        &Default::default(),
    )
    .unwrap();

    let incomplete = models
        .iter()
        .find(|model| model.upstream_model_id == "incomplete-model")
        .unwrap();
    assert!(incomplete.selectable);
    assert_eq!(incomplete.capabilities.vision.value, None);
    assert_eq!(incomplete.reason, None);
    let ready = models
        .iter()
        .find(|model| model.upstream_model_id == "ready-model")
        .unwrap();
    assert!(ready.selectable);
    assert_eq!(ready.reason, None);
}

#[test]
fn unknown_observed_text_model_gets_only_the_marked_conservative_facts() {
    let candidate = candidate_models(
        "candidate/native/runtime-fallback",
        &[],
        &["provider-new-text-model".into()],
        true,
        true,
        &Default::default(),
    )
    .unwrap()
    .pop()
    .unwrap();

    assert!(candidate.selectable);
    assert_eq!(candidate.catalog_configuration_id, None);
    assert_eq!(candidate.membership, ComputeModelMembershipV2::Observed);
    assert_eq!(candidate.capabilities.tool.value, None);
    assert_eq!(candidate.capabilities.vision.value, None);
    assert_eq!(candidate.capabilities.streaming.value, None);
    assert_eq!(candidate.capabilities.context_tokens.value, None);
    assert_eq!(candidate.capabilities.max_output_tokens.value, None);
    assert_eq!(candidate.capabilities.native_reasoning.value, None);
    assert_eq!(
        candidate.capabilities.tool.basis,
        ComputeCandidateFactBasisV2::Unknown
    );
}

#[test]
fn catalog_denied_unknown_model_stays_unselectable() {
    let denied = std::collections::BTreeSet::from(["gpt-image-2".to_owned()]);
    let candidate = candidate_models(
        "candidate/native/non-text",
        &[],
        &["gpt-image-2".into()],
        true,
        false,
        &denied,
    )
    .unwrap()
    .pop()
    .unwrap();

    assert!(!candidate.selectable);
    assert_eq!(
        candidate.reason.as_deref(),
        Some("model_connections.runtime_fallback_ineligible")
    );
    assert_eq!(
        candidate.capabilities.tool.basis,
        ComputeCandidateFactBasisV2::Unknown
    );
}
