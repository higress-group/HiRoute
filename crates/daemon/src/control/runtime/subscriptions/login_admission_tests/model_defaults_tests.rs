//! Persisted custom defaults must survive real connector-owned candidate construction.
use super::*;
use hiroute_application::compiler::CandidateCompilationFactV1;
use hiroute_application::control::RoutingFactsPort;
use hiroute_domain::{
    CanonicalDigest, GatewayCriticalFactV1, GatewayNativeReasoningRenderV1, ModelDefaultFieldV1,
    ModelDefaultPreferenceV1, ModelDefaultValueV1, NativeReasoningCapabilityV1,
};
use hiroute_integrations::TrustedReleaseCatalog;

#[test]
fn qualified_claude_custom_fixed_remains_routable_through_cpa() {
    if crate::test_support::isolated_agent_home(concat!(
        "control::runtime::subscriptions::login_admission_tests::model_defaults_tests::",
        "qualified_claude_custom_fixed_remains_routable_through_cpa"
    )) {
        return;
    }
    let fixture = Fixture::new();
    let login = fixture.authorize(SubscriptionLoginProviderV1::Claude);
    let checked = fixture.check(&login);
    fixture.save(fixture.current_candidate_request(&checked, "claude-custom-initial"));
    let before = fixture.snapshot();
    let source = &before.sources[0];
    let binding = &source.models[0].binding_id;
    let fixed = NativeReasoningCapabilityV1::Fixed {
        profile: "provider-default".into(),
    };
    let old = fixture.saved_request(
        &checked,
        ComputeManagementIntentV2::SaveReady,
        "claude-custom-template",
    );
    let change = hiroute_application_api::saved_compute_change(old.spec.desired_state).unwrap();
    let preview = fixture
        .adapter()
        .preview_compute_save_v3(hiroute_application_api::ComputeManagementChangeV3 {
            schema: hiroute_domain::COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V3.into(),
            change,
            api_link: None,
            default_preferences: [(
                binding.clone(),
                [(
                    ModelDefaultFieldV1::NativeReasoning,
                    ModelDefaultPreferenceV1::Custom {
                        value: ModelDefaultValueV1::Reasoning(fixed.clone()),
                    },
                )]
                .into_iter()
                .collect(),
            )]
            .into_iter()
            .collect(),
        })
        .unwrap();
    fixture.save(ComputeConnectionApplyRequestV1 {
        spec: preview.spec,
        accept_digest: preview.accept_digest,
        expected_revisions: preview.expected_revisions,
        idempotency_key: "claude-custom-fixed".into(),
    });
    let snapshot = fixture
        .adapter()
        .routing_compilation_snapshot(&WorkspaceId::default())
        .unwrap();
    let candidate = snapshot
        .facts
        .candidates
        .iter()
        .find(|c| &c.binding.binding_id == binding)
        .expect("qualified connector-owned candidate must survive a valid custom fixed choice");
    assert!(candidate.is_routable());
    assert_eq!(candidate.reasoning, fixed);
    candidate.validate().unwrap();
    assert_eq!(
        fixture.snapshot().sources[0].models,
        before.sources[0].models
    );
}

const ASTRA: &str = "gpt-6.2-astra";
const SOL: &str = "gpt-6.2-sol";

fn pre_gpt62_catalog(current: &TrustedReleaseCatalog) -> TrustedReleaseCatalog {
    // Model a client whose validated executable catalog predates these entries.
    let keep = |id: &str| {
        !matches!(
            id,
            "model.openai.gpt-6.2-astra" | "model.openai.gpt-6.2-sol"
        )
    };
    let mut bundle = current.current_release_model_data().clone();
    bundle
        .data
        .models
        .retain(|entry| keep(&entry.model_configuration_id));
    bundle
        .data
        .model_endpoint_capabilities
        .retain(|entry| keep(&entry.model_configuration_id));
    for offer in &mut bundle.data.offers {
        offer.model_configuration_ids.retain(|id| keep(id));
    }
    let ratings = &mut bundle.rating_snapshot;
    ratings
        .models
        .retain(|entry| keep(&entry.model_configuration_id));
    ratings
        .records
        .retain(|entry| keep(&entry.model_configuration_id));
    ratings.model_catalog_digest = CanonicalDigest::of(&bundle.data.models).unwrap();
    ratings.digest = ratings.computed_digest().unwrap();

    let registry = serde_json::to_vec(current.registry()).unwrap();
    let model_data = serde_json::to_vec(&bundle).unwrap();
    let mut manifest = current.release_facts_manifest().clone();
    manifest.connector_registry_digest = CanonicalDigest::of_bytes(&registry);
    manifest.model_data_digest = CanonicalDigest::of_bytes(&model_data);
    manifest.cross_reference_digest = bundle.cross_reference_digest(current.registry()).unwrap();
    let manifest = serde_json::to_vec(&manifest).unwrap();
    TrustedReleaseCatalog::load_bundled_release_facts(&manifest, &manifest, &registry, &model_data)
        .unwrap()
}

fn save_gpt62_subscription(fixture: &Fixture) -> ComputeSubscriptionCheckResultV2 {
    fs::write(
        fixture.root.path().join("catalog-model"),
        format!("{ASTRA},{SOL}"),
    )
    .unwrap();
    let login = fixture.authorize(SubscriptionLoginProviderV1::Codex);
    let checked = fixture.check(&login);
    fixture.save(fixture.candidate_request(&checked, "gpt62-subscription"));
    checked
}

fn gpt62_candidates(fixture: &Fixture) -> Vec<CandidateCompilationFactV1> {
    fixture
        .adapter()
        .routing_compilation_snapshot(&WorkspaceId::default())
        .unwrap()
        .facts
        .candidates
}

#[test]
fn existing_codex_fallback_gains_catalog_reasoning_and_keeps_custom_defaults_after_reopen() {
    if crate::test_support::isolated_agent_home(concat!(
        "control::runtime::subscriptions::login_admission_tests::model_defaults_tests::",
        "existing_codex_fallback_gains_catalog_reasoning_and_keeps_custom_defaults_after_reopen"
    )) {
        return;
    }
    let current = crate::release_catalog::fixture_catalog();
    let root = Arc::new(crate::test_support::private_tempdir());
    let before = Fixture::open(root.clone(), false, pre_gpt62_catalog(&current));
    let checked = save_gpt62_subscription(&before);
    let snapshot = before.snapshot();
    let models = &snapshot.sources[0].models;
    assert_eq!(models.len(), 2);
    assert!(
        models
            .iter()
            .all(|model| model.catalog_configuration_id.is_none())
    );
    let astra = models
        .iter()
        .find(|model| model.upstream_model_id == ASTRA)
        .unwrap();
    let fixed = NativeReasoningCapabilityV1::Fixed {
        profile: "provider-default".into(),
    };
    let request = before.saved_request(
        &checked,
        ComputeManagementIntentV2::SaveReady,
        "gpt62-defaults",
    );
    let preview = before
        .adapter()
        .preview_compute_save_v3(hiroute_application_api::ComputeManagementChangeV3 {
            schema: hiroute_domain::COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V3.into(),
            change: hiroute_application_api::saved_compute_change(request.spec.desired_state)
                .unwrap(),
            api_link: None,
            default_preferences: std::collections::BTreeMap::from([(
                astra.binding_id.clone(),
                std::collections::BTreeMap::from([(
                    ModelDefaultFieldV1::NativeReasoning,
                    ModelDefaultPreferenceV1::Custom {
                        value: ModelDefaultValueV1::Reasoning(fixed.clone()),
                    },
                )]),
            )]),
        })
        .unwrap();
    before.save(ComputeConnectionApplyRequestV1 {
        spec: preview.spec,
        accept_digest: preview.accept_digest,
        expected_revisions: preview.expected_revisions,
        idempotency_key: "gpt62-custom".into(),
    });
    let stored = before.snapshot().sources;
    let old_candidates = gpt62_candidates(&before);
    assert_eq!(old_candidates.len(), 2);
    assert!(old_candidates.iter().all(|candidate| matches!(
        candidate.reasoning,
        NativeReasoningCapabilityV1::Fixed { .. }
    )));
    drop(before);

    let after = Fixture::open(root, false, current);
    assert_eq!(after.snapshot().sources, stored);
    let candidates = gpt62_candidates(&after);
    assert_eq!(candidates.len(), 2);
    for old in &old_candidates {
        let candidate = candidates
            .iter()
            .find(|candidate| candidate.binding.binding_id == old.binding.binding_id)
            .unwrap();
        assert!(candidate.is_routable());
        assert_eq!(
            candidate.model.model_configuration_id,
            old.model.model_configuration_id
        );
        if candidate.binding.upstream_model_id == ASTRA {
            assert_eq!(candidate.reasoning, fixed);
            assert!(candidate.protocol_profiles.iter().all(|profile| matches!(
                profile.capability.reasoning_profiles.as_slice(),
                [reasoning] if reasoning.render == GatewayNativeReasoningRenderV1::NoControlParameter
            )));
        } else {
            assert_eq!(candidate.binding.upstream_model_id, SOL);
            assert!(
                matches!(&candidate.reasoning, NativeReasoningCapabilityV1::Discrete { profiles, .. }
                if profiles == &["low", "medium", "high", "xhigh", "max"])
            );
        }
    }
}

#[test]
fn registered_codex_gpt62_profiles_use_separate_input_output_and_total_limits() {
    if crate::test_support::isolated_agent_home(concat!(
        "control::runtime::subscriptions::login_admission_tests::model_defaults_tests::",
        "registered_codex_gpt62_profiles_use_separate_input_output_and_total_limits"
    )) {
        return;
    }
    let fixture = Fixture::new();
    save_gpt62_subscription(&fixture);
    let candidates = gpt62_candidates(&fixture);
    for model in [ASTRA, SOL] {
        let candidate = candidates
            .iter()
            .find(|candidate| candidate.binding.upstream_model_id == model)
            .unwrap();
        assert!(candidate.is_routable());
        assert_eq!(
            candidate.model.model_configuration_id,
            format!("model.openai.{model}")
        );
        for profile in &candidate.protocol_profiles {
            let context = &profile.capability.context;
            assert_eq!(
                context.max_input_tokens,
                GatewayCriticalFactV1::Exact(922_000)
            );
            assert_eq!(
                context.max_output_tokens,
                GatewayCriticalFactV1::Exact(128_000)
            );
            assert_eq!(
                context.max_total_tokens,
                GatewayCriticalFactV1::Exact(Some(1_050_000))
            );
        }
    }
}
