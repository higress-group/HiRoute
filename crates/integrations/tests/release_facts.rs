use hiroute_domain::{CanonicalDigest, ObservedModelV1, ReleaseFactsManifestV2};
use hiroute_integrations::{
    MAX_RELEASE_FACTS_MANIFEST_BYTES, ReleaseVerificationError, TrustedReleaseCatalog,
    builtin_agent_profiles_artifact,
};

const MANIFEST: &[u8] =
    include_bytes!("../../../assets/release-facts/current/bundle/manifest.json");
const REGISTRY: &[u8] =
    include_bytes!("../../../assets/release-facts/current/bundle/connector-registry.json");
const MODEL_DATA: &[u8] =
    include_bytes!("../../../assets/release-facts/current/bundle/model-data.json");

fn current_catalog() -> TrustedReleaseCatalog {
    TrustedReleaseCatalog::load_bundled_release_facts(MANIFEST, MANIFEST, REGISTRY, MODEL_DATA)
        .unwrap()
}

#[test]
fn current_client_bundled_catalog_loads_with_complete_provenance() {
    let catalog = current_catalog();
    let manifest = catalog.release_facts_manifest();
    assert_eq!(manifest.catalog_id, "client-bundled/current");
    assert_eq!(manifest.product_release, "mvp-current");
    assert_eq!(catalog.registry_provenance().0, manifest.catalog_id);
    assert_eq!(catalog.model_data_provenance().0, manifest.catalog_id);
    assert_eq!(
        catalog.current_release_model_data().data.product_release,
        "mvp-current"
    );
    assert!(!catalog.model_metadata().model_records.is_empty());
}

#[test]
fn exact_manifest_pin_and_resource_digests_fail_closed() {
    let mut changed_manifest: ReleaseFactsManifestV2 = serde_json::from_slice(MANIFEST).unwrap();
    changed_manifest.sequence += 1;
    let changed_manifest = serde_json::to_vec(&changed_manifest).unwrap();
    assert!(matches!(
        TrustedReleaseCatalog::load_bundled_release_facts(
            MANIFEST,
            &changed_manifest,
            REGISTRY,
            MODEL_DATA,
        ),
        Err(ReleaseVerificationError::ManifestMismatch)
    ));

    let mut changed_registry = REGISTRY.to_vec();
    changed_registry.push(b' ');
    assert!(matches!(
        TrustedReleaseCatalog::load_bundled_release_facts(
            MANIFEST,
            MANIFEST,
            &changed_registry,
            MODEL_DATA,
        ),
        Err(ReleaseVerificationError::DigestMismatch)
    ));
}

#[test]
fn current_catalog_rejects_malformed_and_oversized_inputs() {
    assert!(matches!(
        TrustedReleaseCatalog::load_release_facts(b"{}", REGISTRY, MODEL_DATA),
        Err(ReleaseVerificationError::MalformedBundle)
            | Err(ReleaseVerificationError::UnsupportedSchema)
    ));
    assert!(matches!(
        TrustedReleaseCatalog::load_release_facts(
            &vec![b' '; MAX_RELEASE_FACTS_MANIFEST_BYTES + 1],
            REGISTRY,
            MODEL_DATA,
        ),
        Err(ReleaseVerificationError::BundleTooLarge)
    ));
}

#[test]
fn current_catalog_reconciles_observed_models_without_cross_provider_matching() {
    let catalog = current_catalog();
    let provider_local_only = catalog
        .reconcile_observed_inventory(
            "endpoint.deepseek.official.global.v1",
            [ObservedModelV1 {
                upstream_model_id: "deepseek-v4-pro".into(),
                metadata: Default::default(),
            }],
        )
        .unwrap();
    assert_eq!(provider_local_only.len(), 1);
    assert_eq!(
        provider_local_only[0].disposition,
        hiroute_domain::InventoryDisposition::InventoryOnly
    );

    let exact_binding = catalog
        .reconcile_observed_inventory(
            "endpoint.bailian.payg.cn.v1",
            [ObservedModelV1 {
                upstream_model_id: "qwen3-max-2026-01-23".into(),
                metadata: Default::default(),
            }],
        )
        .unwrap();
    assert_eq!(
        exact_binding[0].disposition,
        hiroute_domain::InventoryDisposition::CatalogMatched
    );
}

#[test]
fn current_codex_subscription_catalog_qualifies_gpt6_on_its_exact_account_inventory() {
    let catalog = current_catalog();
    for (upstream_model_id, profiles) in [
        (
            "gpt-6-astra",
            &["low", "medium", "high", "xhigh", "max"][..],
        ),
        (
            "gpt-6-sol",
            &["none", "low", "medium", "high", "xhigh", "max"][..],
        ),
        (
            "gpt-6.1-sol",
            &["low", "medium", "high", "xhigh", "max"][..],
        ),
        (
            "gpt-6-luna",
            &["none", "low", "medium", "high", "xhigh", "max"][..],
        ),
    ] {
        let models = catalog
            .reconcile_observed_inventory(
                "endpoint.cpa.codex",
                [ObservedModelV1 {
                    upstream_model_id: upstream_model_id.into(),
                    metadata: Default::default(),
                }],
            )
            .unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(
            models[0].disposition,
            hiroute_domain::InventoryDisposition::CatalogMatched
        );
        let configuration_id = format!("model.openai.{upstream_model_id}");
        assert_eq!(
            models[0].model_configuration_id.as_deref(),
            Some(configuration_id.as_str())
        );
        let definition = catalog.model_data().model(&configuration_id).unwrap();
        assert_eq!(definition.capabilities.context_tokens, 1_050_000);
        assert_eq!(definition.capabilities.max_output_tokens, 128_000);
        let other_provider = catalog
            .reconcile_observed_inventory(
                "endpoint.bailian.payg.cn.v1",
                [ObservedModelV1 {
                    upstream_model_id: upstream_model_id.into(),
                    metadata: Default::default(),
                }],
            )
            .unwrap();
        assert_eq!(
            other_provider[0].disposition,
            hiroute_domain::InventoryDisposition::InventoryOnly
        );
        assert_eq!(
            catalog
                .native_reasoning()
                .iter()
                .find(|value| value.model_configuration_id == configuration_id)
                .map(|value| &value.capability),
            Some(&hiroute_domain::NativeReasoningCapabilityV1::Discrete {
                parameter: "reasoning_effort".into(),
                profiles: profiles
                    .iter()
                    .map(|profile| (*profile).to_owned())
                    .collect(),
                default_profile: None,
            })
        );
    }
}

#[test]
fn current_metadata_bounds_the_unknown_text_fallback_without_inventing_identity() {
    let catalog = current_catalog();
    assert!(catalog.runtime_fallback_allows_observed_text("provider-new-text-model"));
    assert!(!catalog.runtime_fallback_allows_observed_text("gpt-image-2"));
    assert!(
        catalog
            .runtime_fallback_denied_model_ids()
            .contains("gpt-image-2")
    );
}

#[test]
fn bundled_agent_profiles_are_deterministic_and_valid() {
    let artifact = builtin_agent_profiles_artifact();
    artifact.validate().unwrap();
    assert_eq!(
        CanonicalDigest::of(&artifact).unwrap(),
        CanonicalDigest::of(&builtin_agent_profiles_artifact()).unwrap()
    );
}
