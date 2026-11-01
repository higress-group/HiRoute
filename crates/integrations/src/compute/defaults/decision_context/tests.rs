use super::*;

fn catalog() -> super::super::TrustedReleaseCatalog {
    super::super::TrustedReleaseCatalog::load_release_facts(
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/release-facts/current/bundle/manifest.json"
        )),
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/release-facts/current/bundle/connector-registry.json"
        )),
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/release-facts/current/bundle/model-data.json"
        )),
    )
    .unwrap()
}

#[test]
fn maintained_bindings_resolve_only_on_the_documented_wire_surface() {
    let catalog = catalog();
    for (endpoint, protocol, model, window, question, basis) in [
        (
            "https://api.typesafe.ai/v1/systemone",
            DecisionWireProtocolV2::SystemOne,
            "jev-1.13.0",
            262144,
            262144,
            DecisionContextBasisV1::Inferred,
        ),
        (
            "https://API.TYPESAFE.AI:443/v1/systemone",
            DecisionWireProtocolV2::SystemOne,
            "jev-1.14.0",
            262144,
            262144,
            DecisionContextBasisV1::Inferred,
        ),
        (
            "https://workspace-123.cn-beijing.maas.aliyuncs.com/compatible-mode/v1/systemone",
            DecisionWireProtocolV2::SystemOne,
            "decision-model-preview",
            65536,
            65536,
            DecisionContextBasisV1::Documented,
        ),
        (
            "https://workspace.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1/systemone",
            DecisionWireProtocolV2::SystemOne,
            "decision-model-preview",
            65536,
            65536,
            DecisionContextBasisV1::Documented,
        ),
        (
            "https://openrouter.ai/api/alpha/decisions",
            DecisionWireProtocolV2::SystemOne,
            "typesafe/jev-1.13",
            262144,
            262144,
            DecisionContextBasisV1::Documented,
        ),
        (
            "https://openrouter.ai/api/alpha/decisions",
            DecisionWireProtocolV2::SystemOne,
            "typesafe/jev-1.14",
            262144,
            262144,
            DecisionContextBasisV1::Documented,
        ),
        (
            "https://api.openai.com/v1/decisions",
            DecisionWireProtocolV2::OpenaiDecisions,
            "gpt-6-luna",
            1050000,
            1050000,
            DecisionContextBasisV1::MaintainerPolicy,
        ),
    ] {
        let resolved = catalog
            .resolve_decision_context(
                endpoint,
                protocol,
                model,
                &DecisionContextPolicyV1::default(),
            )
            .unwrap();
        assert!(resolved.validate());
        assert_eq!(
            (
                resolved.request_window_tokens,
                resolved.per_question_window_tokens
            ),
            (window, question)
        );
        assert_eq!(resolved.request_source, DecisionContextSourceV1::Metadata);
        assert_eq!(resolved.metadata.unwrap().basis, basis);
        assert_eq!(
            resolved.typesafe_long_context_header,
            endpoint.to_lowercase().contains("api.typesafe.ai")
        );
    }
}

#[test]
fn unmatched_models_aliases_codecs_and_destinations_use_fallback() {
    let catalog = catalog();
    for (endpoint, protocol, model) in [
        (
            "https://unknown.example/v1/decisions",
            DecisionWireProtocolV2::OpenaiDecisions,
            "gpt-6-luna",
        ),
        (
            "https://api.openai.com/v1/responses",
            DecisionWireProtocolV2::OpenaiDecisions,
            "gpt-6-luna",
        ),
        (
            "https://api.openai.com/v1/decisions",
            DecisionWireProtocolV2::SystemOne,
            "gpt-6-luna",
        ),
        (
            "https://api.typesafe.ai/v1/systemone",
            DecisionWireProtocolV2::SystemOne,
            "jev-latest",
        ),
        (
            "https://api.typesafe.ai.example/v1/systemone",
            DecisionWireProtocolV2::SystemOne,
            "jev-1.14.0",
        ),
        (
            "https://openrouter.ai/api/v1/decisions",
            DecisionWireProtocolV2::SystemOne,
            "typesafe/jev-1.14",
        ),
        (
            "https://x.y.cn-beijing.maas.aliyuncs.com/compatible-mode/v1/systemone",
            DecisionWireProtocolV2::SystemOne,
            "decision-model-preview",
        ),
        (
            "https://workspace.cn-beijing.maas.aliyuncs.com.example/compatible-mode/v1/systemone",
            DecisionWireProtocolV2::SystemOne,
            "decision-model-preview",
        ),
    ] {
        let resolved = catalog
            .resolve_decision_context(
                endpoint,
                protocol,
                model,
                &DecisionContextPolicyV1::default(),
            )
            .unwrap();
        assert!(resolved.validate());
        assert_eq!(resolved.request_window_tokens, 32000);
        assert_eq!(resolved.request_source, DecisionContextSourceV1::Fallback);
        assert!(resolved.metadata.is_none());
    }
}

#[test]
fn header_mode_precedes_limits_and_user_request_does_not_erase_question_cap() {
    let catalog = catalog();
    let endpoint = "https://api.typesafe.ai/v1/systemone";
    let protocol = DecisionWireProtocolV2::SystemOne;
    let mut policy = DecisionContextPolicyV1 {
        typesafe_long_context: TypeSafeLongContextV1::Disabled,
        ..Default::default()
    };
    let automatic = catalog
        .resolve_decision_context(endpoint, protocol, "jev-1.14.0", &policy)
        .unwrap();
    assert_eq!(
        (
            automatic.request_window_tokens,
            automatic.per_question_window_tokens
        ),
        (64000, 32000)
    );
    assert!(!automatic.typesafe_long_context_header);
    policy.mode = DecisionContextModeV1::Custom;
    policy.window_tokens = Some(64000);
    let custom = catalog
        .resolve_decision_context(endpoint, protocol, "jev-1.14.0", &policy)
        .unwrap();
    assert_eq!(custom.request_source, DecisionContextSourceV1::User);
    assert_eq!(
        custom.per_question_source,
        DecisionContextSourceV1::Metadata
    );
    assert_eq!(custom.per_question_window_tokens, 32000);
    assert_ne!(custom.context_digest, automatic.context_digest);
    policy.window_tokens = Some(16000);
    let clamped = catalog
        .resolve_decision_context(endpoint, protocol, "jev-1.14.0", &policy)
        .unwrap();
    assert_eq!(clamped.per_question_window_tokens, 16000);
    policy.window_tokens = Some(100000);
    policy.per_question_tokens = Some(90000);
    let override_both = catalog
        .resolve_decision_context(endpoint, protocol, "jev-1.14.0", &policy)
        .unwrap();
    assert_eq!(override_both.per_question_window_tokens, 90000);
    assert_eq!(
        override_both.per_question_source,
        DecisionContextSourceV1::User
    );
    let mut forged = override_both.clone();
    forged.request_window_tokens += 1;
    assert!(!forged.validate());
}

#[test]
fn index_does_not_promote_decisions_to_inference_and_rejects_ambiguous_facts() {
    let mut catalog = catalog();
    assert!(
        catalog
            .model_data()
            .models
            .iter()
            .all(|model| !model.model_configuration_id.contains("jev"))
    );
    let metadata = &mut catalog.current_release_model_data.metadata_catalog;
    let index = metadata
        .endpoint_bindings
        .iter()
        .position(|binding| binding.upstream_model_id == "jev-1.14.0")
        .unwrap();
    let mut duplicate = metadata.endpoint_bindings[index].clone();
    duplicate.binding_key += "/duplicate";
    metadata.endpoint_bindings.push(duplicate);
    let result = catalog.resolve_decision_context(
        "https://api.typesafe.ai/v1/systemone",
        DecisionWireProtocolV2::SystemOne,
        "jev-1.14.0",
        &DecisionContextPolicyV1::default(),
    );
    assert_eq!(
        result,
        Err(DecisionContextResolutionError::AmbiguousMetadata)
    );
}

#[test]
fn attaching_a_new_catalog_discards_the_previously_built_index() {
    let mut catalog = catalog();
    let policy = DecisionContextPolicyV1::default();
    let before = catalog
        .resolve_decision_context(
            "https://api.openai.com/v1/decisions",
            DecisionWireProtocolV2::OpenaiDecisions,
            "gpt-6-luna",
            &policy,
        )
        .unwrap();
    let snapshot = catalog.official_snapshot();
    catalog = catalog.with_embedded_official_data(snapshot).unwrap();
    assert!(catalog.decision_context_index.get().is_none());
    let after = catalog
        .resolve_decision_context(
            "https://api.openai.com/v1/decisions",
            DecisionWireProtocolV2::OpenaiDecisions,
            "gpt-6-luna",
            &policy,
        )
        .unwrap();
    assert_eq!(before, after);
}
