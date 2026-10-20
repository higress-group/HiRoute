//! Shared public settings transactions with OpenCode's loaded user layers and real recovery.
use super::models::{model_spec, model_status, restore_spec};
use super::*;

fn configure_models(fixture: &QoderFixture) {
    let mut spec = model_spec(fixture);
    spec["model"]["settings"]["mode"] = json!("opencode_additional");
    apply(&fixture.service, &fixture.runtime, spec, "opencode-models");
}

fn selector(fixture: &QoderFixture) -> String {
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(&fixture.native_config).unwrap()).unwrap();
    let (provider, declaration) = value["provider"]
        .as_object()
        .unwrap()
        .iter()
        .next()
        .unwrap();
    format!(
        "{provider}/{}",
        declaration["models"]
            .as_object()
            .unwrap()
            .keys()
            .next()
            .unwrap()
    )
}

#[test]
fn opencode_markdown_references_block_restore_but_keep_rotation_and_user_resources() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::qoder::opencode::opencode_markdown_references_block_restore_but_keep_rotation_and_user_resources",
    ) {
        return;
    }
    let fixture = QoderFixture::with_kind(true, AgentKindV1::OpenCode);
    configure_models(&fixture);
    for directory in ["agent/nested", "agents", "mode", "modes"] {
        let path = fixture
            .native_config
            .parent()
            .unwrap()
            .join(directory)
            .join("reviewer.md");
        let content = format!(
            "---\nmodel: '{}'\n---\nKeep this native prompt.\n",
            selector(&fixture)
        );
        private_file(&path, content.as_bytes());
        let before = fs::read(&fixture.native_config).unwrap();
        let status = model_status(&fixture);
        let preview = fixture.service.dispatch_wire(request(
            "PreviewAgentConnectionRestore",
            json!({"spec":restore_spec(&fixture, &status)}),
            None,
        ));
        assert!(preview.error.is_none(), "{preview:?}");
        let preview = preview.data.unwrap();
        assert_eq!(preview["applicable"], false, "{preview}");
        assert!(
            preview["blockers"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["reason"] == "additional_default_in_use")
        );
        assert_eq!(fs::read(&fixture.native_config).unwrap(), before);
        assert_eq!(model_status(&fixture)["state"], "configured");
        let mut rotation = model_spec(&fixture);
        rotation["model"]["settings"]["mode"] = json!("opencode_additional");
        rotation["access_token"] = json!({"intent":"regenerate"});
        apply(
            &fixture.service,
            &fixture.runtime,
            rotation,
            &format!("markdown-rotation-{}", directory.replace('/', "-")),
        );
        assert_eq!(fs::read(&path).unwrap(), content.as_bytes());
        fs::remove_file(path).unwrap();
    }
    // Unknown frontmatter has the same removal boundary, without blocking rotation.
    let path = fixture
        .native_config
        .parent()
        .unwrap()
        .join("agents/unknown.md");
    private_file(&path, b"---\n<<: {model: hidden/route}\n---\nKeep\n");
    let status = model_status(&fixture);
    let preview = fixture
        .service
        .dispatch_wire(request(
            "PreviewAgentConnectionRestore",
            json!({"spec":restore_spec(&fixture, &status)}),
            None,
        ))
        .data
        .unwrap();
    assert_eq!(preview["applicable"], false);
    let mut rotation = model_spec(&fixture);
    rotation["model"]["settings"]["mode"] = json!("opencode_additional");
    apply(
        &fixture.service,
        &fixture.runtime,
        rotation,
        "markdown-unknown-retained",
    );
    fs::remove_file(path).unwrap();
    apply(
        &fixture.service,
        &fixture.runtime,
        restore_spec(&fixture, &model_status(&fixture)),
        "markdown-safe-restore",
    );
    assert_eq!(model_status(&fixture)["state"], "not_configured");
}

#[test]
fn opencode_markdown_added_or_changed_after_preview_rejects_stale_restore() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::qoder::opencode::opencode_markdown_added_or_changed_after_preview_rejects_stale_restore",
    ) {
        return;
    }
    let fixture = QoderFixture::with_kind(true, AgentKindV1::OpenCode);
    configure_models(&fixture);
    let path = fixture
        .native_config
        .parent()
        .unwrap()
        .join("agents/reviewer.md");
    let reference = format!("---\nmodel: '{}'\n---\nNative prompt\n", selector(&fixture));
    for existed in [false, true] {
        if existed {
            private_file(&path, b"---\nmodel: daily/default\n---\nNative prompt\n");
        }
        let spec = restore_spec(&fixture, &model_status(&fixture));
        let preview = fixture
            .service
            .dispatch_wire(request(
                "PreviewAgentConnectionRestore",
                json!({"spec":spec}),
                None,
            ))
            .data
            .unwrap();
        assert_eq!(preview["applicable"], true);
        let before = fs::read(&fixture.native_config).unwrap();
        private_file(&path, reference.as_bytes());
        let response = fixture.service.dispatch_wire(request("ApplyAgentConnectionRestore", json!({"spec":spec,
            "accept_digest":preview["accept_digest"],"dependency_digest":preview["dependency_digest"],
            "expected_revisions":preview["expected_revisions"],"idempotency_key":format!("markdown-stale-{existed}")}), None));
        assert_eq!(
            response.error.unwrap().code,
            api::ErrorCode::ChangePreviewStale
        );
        assert_eq!(fs::read(&fixture.native_config).unwrap(), before);
        assert_eq!(model_status(&fixture)["state"], "configured");
        fs::remove_file(&path).unwrap();
    }
}

#[test]
fn opencode_markdown_reference_added_during_staging_preserves_the_active_route() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::qoder::opencode::opencode_markdown_reference_added_during_staging_preserves_the_active_route",
    ) {
        return;
    }
    let fixture = QoderFixture::with_kind(true, AgentKindV1::OpenCode);
    configure_models(&fixture);
    let path = fixture
        .native_config
        .parent()
        .unwrap()
        .join("agents/reviewer.md");
    private_file(&path, b"Native prompt without a model\n");
    let reference = format!("---\nmodel: '{}'\n---\nNative prompt\n", selector(&fixture));
    let spec = restore_spec(&fixture, &model_status(&fixture));
    let before = fs::read(&fixture.native_config).unwrap();
    let mut ports = fixture.runtime.application_ports();
    ports.mutation = Some(Arc::new(fault::FileRace {
        adapter: fixture.runtime.adapter.clone(),
        path: path.clone(),
        fail_after_file: false,
        replacement: Some(reference.as_bytes().to_vec()),
    }));
    let racing = LocalControlDaemon::new(ApplicationService::new(ports));
    let preview = racing
        .dispatch_wire(request(
            "PreviewAgentConnectionRestore",
            json!({"spec":spec}),
            None,
        ))
        .data
        .unwrap();
    assert_eq!(preview["applicable"], true);
    let response = racing.dispatch_wire(request("ApplyAgentConnectionRestore", json!({"spec":spec,
        "accept_digest":preview["accept_digest"],"dependency_digest":preview["dependency_digest"],
        "expected_revisions":preview["expected_revisions"],"idempotency_key":"markdown-staging-race"}), None));
    assert!(response.error.is_none(), "{response:?}");
    assert_eq!(response.data.unwrap()["state"], "rolled_back");
    assert_eq!(fs::read(&fixture.native_config).unwrap(), before);
    assert_eq!(fs::read(path).unwrap(), reference.as_bytes());
    assert_eq!(model_status(&fixture)["state"], "configured");
}

#[test]
fn opencode_pending_file_tail_retries_the_original_context_bound_operation() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::qoder::opencode::opencode_pending_file_tail_retries_the_original_context_bound_operation",
    ) {
        return;
    }
    super::models::pending_file_tail_retries_exact_operation(AgentKindV1::OpenCode);
}

#[test]
fn opencode_selected_native_default_blocks_restore_with_an_actionable_preview() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::qoder::opencode::opencode_selected_native_default_blocks_restore_with_an_actionable_preview",
    ) {
        return;
    }
    let fixture = QoderFixture::with_kind(true, AgentKindV1::OpenCode);
    configure_models(&fixture);
    let status = model_status(&fixture);
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&fixture.native_config).unwrap()).unwrap();
    let (provider, declaration) = value["provider"]
        .as_object()
        .unwrap()
        .iter()
        .next()
        .unwrap();
    let model = declaration["models"]
        .as_object()
        .unwrap()
        .keys()
        .next()
        .unwrap();
    value["model"] = json!(format!("{provider}/{model}"));
    private_file(&fixture.native_config, &serde_json::to_vec(&value).unwrap());
    let before = fs::read(&fixture.native_config).unwrap();
    let preview = fixture.service.dispatch_wire(request(
        "PreviewAgentConnectionRestore",
        json!({"spec":restore_spec(&fixture, &status)}),
        None,
    ));
    assert!(preview.error.is_none(), "{preview:?}");
    let preview = preview.data.unwrap();
    assert_eq!(preview["applicable"], false, "{preview:?}");
    assert!(
        preview["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["reason"] == "additional_default_in_use"),
        "{preview:?}"
    );
    assert_eq!(fs::read(&fixture.native_config).unwrap(), before);
}

#[test]
fn opencode_review_new_jsonc_keeps_managed_json_binding_after_restart_and_restore() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::qoder::opencode::opencode_review_new_jsonc_keeps_managed_json_binding_after_restart_and_restore",
    ) {
        return;
    }
    let mut fixture = QoderFixture::with_kind(true, AgentKindV1::OpenCode);
    let original = fs::read(&fixture.native_config).unwrap();
    configure_models(&fixture);
    let status = model_status(&fixture);
    let managed = fs::read(&fixture.native_config).unwrap();
    let added = fixture.native_config.with_extension("jsonc");
    let unrelated = br#"{"$schema":"https://opencode.ai/config.json","instructions":["new.md"]}"#;
    private_file(&added, unrelated);
    let storage = fixture._root.path().join("storage");
    let home = PathBuf::from(std::env::var_os("HOME").unwrap());
    let mut layout = AgentFilesystemLayoutV1::from_process(&home, fixture._root.path());
    layout.opencode_context =
        Some(hiroute_integrations::OpenCodeNativeContext::from_lookup(&home, |_| None).unwrap());
    let registry = serde_json::from_slice(include_bytes!(
        "../../../../../assets/connector-registry/current/registry-seed.json"
    ))
    .unwrap();
    let model_data: hiroute_domain::ReleaseModelDataBundleV2 = serde_json::from_slice(
        include_bytes!("../../../../../assets/release-facts/current/bundle/model-data.json"),
    )
    .unwrap();
    let scanner = FilesystemAgentScannerV1::new(
        layout,
        ClaudeRegistrationIndexV1::from_verified_model_data(&registry, &model_data.data).unwrap(),
    );
    drop(fixture.service);
    drop(fixture.runtime);
    fixture.runtime = open_with_scanner(&storage, scanner);
    fixture
        .runtime
        .adapter
        .reconcile_startup_and_open()
        .unwrap();
    fixture.service = LocalControlDaemon::new(ApplicationService::new(
        fixture
            .runtime
            .application_ports()
            .with_agent_connection(Arc::new(FixtureFacts::collaboration(
                fixture.runtime.adapter.clone(),
            ))),
    ));
    assert_eq!(
        fixture
            .runtime
            .adapter
            .scanner
            .opencode_user_models_target(),
        fixture.native_config
    );
    assert_eq!(fs::read(&fixture.native_config).unwrap(), managed);
    assert_eq!(model_status(&fixture)["state"], "configured");
    apply(
        &fixture.service,
        &fixture.runtime,
        restore_spec(&fixture, &status),
        "opencode-restarted-restore",
    );
    assert_eq!(model_status(&fixture)["state"], "not_configured");
    assert_eq!(fs::read(&fixture.native_config).unwrap(), original);
    assert_eq!(fs::read(added).unwrap(), unrelated);
}

#[test]
fn opencode_review_status_reports_relevant_user_layer_drift_only() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::qoder::opencode::opencode_review_status_reports_relevant_user_layer_drift_only",
    ) {
        return;
    }
    let fixture = QoderFixture::with_kind(true, AgentKindV1::OpenCode);
    configure_models(&fixture);
    let original_status = model_status(&fixture);
    let managed = fs::read(&fixture.native_config).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&managed).unwrap();
    let provider = value["provider"]
        .as_object()
        .unwrap()
        .keys()
        .next()
        .unwrap();
    let overlay = PathBuf::from(std::env::var_os("HOME").unwrap()).join(".opencode/opencode.jsonc");
    let unrelated = br#"{"$schema":"https://opencode.ai/config.json","instructions":["other.md"],"provider":{"daily":{}}}"#;
    for conflict in [
        json!({"provider":{(provider):{"options":{"baseURL":"https://example.invalid/v1"}}}}),
        json!({"enabled_providers":["daily"]}),
        json!({"disabled_providers":[provider]}),
    ] {
        private_file(&overlay, unrelated);
        assert_eq!(model_status(&fixture)["state"], "configured");
        private_file(&overlay, &serde_json::to_vec(&conflict).unwrap());
        let status = model_status(&fixture);
        assert_eq!(status["state"], "drift", "{conflict}");
        assert_eq!(
            status["restore_point_ref"],
            original_status["restore_point_ref"]
        );
        assert_eq!(fs::read(&fixture.native_config).unwrap(), managed);
    }
    private_file(&overlay, unrelated);
    assert_eq!(model_status(&fixture)["state"], "configured");
}
