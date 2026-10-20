use super::*;

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
