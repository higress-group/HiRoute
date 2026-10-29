//! Normal settings install/restore must keep Desktop and Web ownership separate.
use super::models::{model_spec, model_status, restore_spec};
use super::*;

#[test]
fn desktop_and_web_models_install_independently_and_restore_after_reopen_without_app() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::qoder::dsh_desktop::desktop_and_web_models_install_independently_and_restore_after_reopen_without_app",
    ) {
        return;
    }
    let mut fixture = QoderFixture::with_desktop_models();
    let web_context = fixture
        .runtime
        .adapter
        .settings_context_for_agent("agent_dsh_default")
        .unwrap();
    let desktop_context = fixture
        .runtime
        .adapter
        .settings_context_for_agent("agent_dsh_desktop")
        .unwrap();
    assert_ne!(web_context, desktop_context);
    let web = fixture.runtime.adapter.scanner.dsh_user_models_target();
    let desktop = fixture.runtime.adapter.scanner.dsh_desktop_models_target();
    let desktop_adapter = fixture
        .runtime
        .adapter
        .scanner
        .dsh_desktop_routing_adapter_target();
    let original_desktop = fs::read(&desktop).unwrap();
    fixture.context = web_context.clone();
    fixture.native_config = web.clone();
    let mut web_spec = model_spec(&fixture);
    web_spec["model"]["settings"]["mode"] = json!("dsh_additional");
    apply(
        &fixture.service,
        &fixture.runtime,
        web_spec,
        "web-configure",
    );
    let configured_web = fs::read(&web).unwrap();
    assert_eq!(fs::read(&desktop).unwrap(), original_desktop);
    let web_adapter = fixture
        .runtime
        .adapter
        .scanner
        .native_routing_adapter_target(AgentKindV1::DeepseekHarness)
        .unwrap();
    let configured_web_adapter = fs::read(&web_adapter).unwrap();

    // Desktop's actual carrier is sufficient. The separately installed CLI is absent.
    fs::remove_file(
        fixture
            .runtime
            .adapter
            .scanner
            .dsh_executable_target()
            .unwrap(),
    )
    .unwrap();
    fixture.context = desktop_context.clone();
    fixture.native_config = desktop.clone();
    let mut spec = model_spec(&fixture);
    spec["model"]["settings"]["mode"] = json!("dsh_additional");
    let scan = fixture.service.dispatch_wire(request(
        "ScanAgents",
        json!({"context_id":desktop_context}),
        None,
    ));
    assert!(scan.error.is_none(), "{scan:?}");
    let rows = scan.data.unwrap();
    assert_eq!(rows["agents"][0]["agent_id"], "agent_dsh_desktop");
    apply(
        &fixture.service,
        &fixture.runtime,
        spec.clone(),
        "desktop-configure",
    );
    assert_eq!(model_status(&fixture)["state"], "configured");
    let configured_desktop = fs::read(&desktop).unwrap();
    let text = String::from_utf8_lossy(&configured_desktop);
    assert!(text.contains("hiroute-routing") && text.contains("llm-pi-ai"));
    assert!(text.contains("profiles/desktop/hiroute-routing.mjs"));
    assert_eq!(
        fs::read(&desktop_adapter).unwrap(),
        hiroute_integrations::DSH_ROUTING_ADAPTER.as_bytes()
    );
    assert_eq!(fs::read(&web).unwrap(), configured_web);
    let fixture = native_routing::reopen(fixture);
    assert_eq!(model_status(&fixture)["state"], "configured");
    fs::remove_file(
        fixture
            .runtime
            .adapter
            .scanner
            .dsh_desktop_executable_target()
            .unwrap(),
    )
    .unwrap();
    let rejected = fixture.service.dispatch_wire(request(
        "PreviewAgentConnectionChange",
        json!({"spec":spec}),
        None,
    ));
    assert!(
        rejected.error.is_some(),
        "missing app must block Configure, not Restore"
    );
    assert_eq!(fs::read(&desktop).unwrap(), configured_desktop);
    apply(
        &fixture.service,
        &fixture.runtime,
        restore_spec(&fixture, &model_status(&fixture)),
        "desktop-restore",
    );
    assert_eq!(model_status(&fixture)["state"], "not_configured");
    assert!(!desktop_adapter.exists());
    assert!(!String::from_utf8_lossy(&fs::read(&desktop).unwrap()).contains("hiroute-routing"));
    assert!(
        fs::read(&desktop)
            .unwrap()
            .starts_with(b"# keep Desktop comment\n")
    );
    assert_eq!(fs::read(&web).unwrap(), configured_web);
    assert_eq!(fs::read(&web_adapter).unwrap(), configured_web_adapter);
}

#[test]
fn desktop_runtime_change_invalidates_preview_even_when_new_composition_is_valid() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::qoder::dsh_desktop::desktop_runtime_change_invalidates_preview_even_when_new_composition_is_valid",
    ) {
        return;
    }
    let mut fixture = QoderFixture::with_desktop_models();
    fixture.context = fixture
        .runtime
        .adapter
        .settings_context_for_agent("agent_dsh_desktop")
        .unwrap();
    fixture.native_config = fixture.runtime.adapter.scanner.dsh_desktop_models_target();
    let mut spec = model_spec(&fixture);
    spec["model"]["settings"]["mode"] = json!("dsh_additional");
    let preview = fixture.service.dispatch_wire(request(
        "PreviewAgentConnectionChange",
        json!({"spec":spec}),
        None,
    ));
    assert!(preview.error.is_none(), "{preview:?}");
    let preview = preview.data.unwrap();
    let before = fs::read(&fixture.native_config).unwrap();
    let executable = fixture
        .runtime
        .adapter
        .scanner
        .dsh_desktop_executable_target()
        .unwrap();
    let archive = executable
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("Resources/app.asar");
    private_file(&archive, b"changed bundled runtime, same advertised module");
    let rejected = fixture.service.dispatch_wire(request("ApplyAgentConnectionChange", json!({"spec":spec,
        "accept_digest":preview["accept_digest"], "dependency_digest":preview["dependency_digest"],
        "expected_revisions":preview["expected_revisions"], "idempotency_key":"desktop-runtime-changed"}), None));
    assert!(
        rejected.error.is_some() && rejected.operation.is_none(),
        "stale runtime proof was accepted: {rejected:?}"
    );
    assert_eq!(fs::read(&fixture.native_config).unwrap(), before);
    apply(
        &fixture.service,
        &fixture.runtime,
        spec.clone(),
        "desktop-runtime-fresh",
    );
    assert_eq!(model_status(&fixture)["state"], "configured");
    let configured = fs::read(&fixture.native_config).unwrap();
    spec["collaboration"] = json!({"intent":"configure","settings":{"trigger_mode":"explicit"}});
    let denied = fixture.service.dispatch_wire(request(
        "PreviewAgentConnectionChange",
        json!({"spec":spec}),
        None,
    ));
    assert!(
        denied.error.is_some(),
        "Desktop model capability must not confer Skill authority"
    );
    assert_eq!(fs::read(&fixture.native_config).unwrap(), configured);
}
