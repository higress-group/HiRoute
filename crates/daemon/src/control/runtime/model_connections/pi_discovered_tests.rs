use super::*;
use hiroute_domain::CanonicalDigest;

#[test]
fn review_native_preview_uses_captured_catalog_for_fallback_denial() {
    #[cfg(unix)]
    if crate::test_support::isolated_agent_home(
        "control::runtime::model_connections::pi_discovered::tests::review_native_preview_uses_captured_catalog_for_fallback_denial",
    ) {
        return;
    }
    let live = crate::release_catalog::fixture_catalog();
    let model = "gpt-image-2";
    assert!(live.runtime_fallback_denied_model_ids().contains(model));

    // A previously captured, fully validated bundle lacks this later metadata.
    // Keep a different current catalog active while preparing with the capture.
    let mut data = live.current_release_model_data().clone();
    data.metadata_catalog
        .model_records
        .retain(|r| r.upstream_model_id != model);
    data.metadata_catalog
        .endpoint_bindings
        .retain(|b| b.upstream_model_id != model);
    let registry = serde_json::to_vec(live.registry()).unwrap();
    let bytes = serde_json::to_vec(&data).unwrap();
    let mut manifest = live.release_facts_manifest().clone();
    manifest.connector_registry_digest = CanonicalDigest::of_bytes(&registry);
    manifest.model_data_digest = CanonicalDigest::of_bytes(&bytes);
    manifest.cross_reference_digest = data.cross_reference_digest(live.registry()).unwrap();
    let captured = TrustedReleaseCatalog::load_release_facts(
        &serde_json::to_vec(&manifest).unwrap(),
        &registry,
        &bytes,
    )
    .unwrap();
    assert!(!captured.runtime_fallback_denied_model_ids().contains(model));

    let root = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let runtime = crate::control::ProductionControlRuntime::open_with_release_catalog(
        root.path(),
        live.clone(),
    )
    .unwrap();
    let path = runtime.adapter.scanner.pi_user_models_target();
    assert!(
        path.starts_with(std::env::var_os("HOME").unwrap()),
        "native fixture must stay in its isolated home"
    );
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, serde_json::to_vec(&serde_json::json!({"providers":{"snapshot-test":{
        "baseUrl":"https://api.example.test/v1","api":"openai-responses","apiKey":"test-only-native-key",
        "models":[{"id":model,"contextWindow":16384,"maxTokens":4096,"input":["text"]}]
    }}})).unwrap()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let source = runtime
        .adapter
        .scanner
        .pi_api_sources()
        .unwrap()
        .into_iter()
        .find(|s| s.model_id == model)
        .unwrap();
    let prepare = |catalog: &TrustedReleaseCatalog, id: &str| {
        runtime
            .adapter
            .prepare_native_source_candidate(
                vec![source.clone()],
                pi_discovery(&source),
                source.evidence_digest.clone(),
                id,
                false,
                catalog,
            )
            .unwrap()
    };
    let old = prepare(&captured, "captured-preview");
    let new = prepare(&live, "current-preview");
    let selectable = |view: &ComputeCandidateViewV2| {
        view.models
            .iter()
            .find(|m| m.upstream_model_id == model)
            .unwrap()
            .selectable
    };
    assert!(
        selectable(&old),
        "captured preview mixed in current denial metadata"
    );
    assert!(
        !selectable(&new),
        "control: current denial must still apply to a new preview"
    );
}
