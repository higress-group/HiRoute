//! Real protected adapter effects and restarted transaction/status readers.
use super::models::{model_spec, model_status, restore_spec};
use super::*;

fn reopen(fixture: QoderFixture) -> QoderFixture {
    let scanner = fixture.runtime.adapter.scanner.clone();
    let QoderFixture {
        _root,
        runtime,
        service,
        context,
        native_config,
        skill,
        untouched,
    } = fixture;
    drop(service);
    drop(runtime);
    let storage = _root.path().join("storage");
    let runtime = ProductionControlRuntime::prepare_for_role_all_with_scanner(
        &storage,
        crate::release_catalog::fixture_catalog(),
        None,
        scanner,
    )
    .unwrap();
    let runtime =
        configure_runtime_with_initial(&storage, runtime, golden_publication_plans_only());
    runtime.adapter.reconcile_startup_and_open().unwrap();
    let service =
        LocalControlDaemon::new(ApplicationService::new(
            runtime.application_ports().with_agent_connection(Arc::new(
                FixtureFacts::collaboration(runtime.adapter.clone()),
            )),
        ));
    QoderFixture {
        _root,
        runtime,
        service,
        context,
        native_config,
        skill,
        untouched,
    }
}

#[test]
fn native_routing_adapter_effects_restore_after_reopen_and_detect_drift() {
    let tools = crate::test_support::private_tempdir();
    let node = tools.path().join("node");
    private_file(&node, b"#!/bin/sh\nif [ \"$1\" = --version ]; then echo v22.19.0; elif [ \"$2\" = --check ] && [ \"$4\" = models ]; then echo hiroute.pi-sdk-capability/v1:ok; else exit 97; fi\n");
    fs::set_permissions(&node, fs::Permissions::from_mode(0o700)).unwrap();
    let path =
        std::env::join_paths([tools.path(), Path::new("/usr/bin"), Path::new("/bin")]).unwrap();
    if crate::test_support::isolated_agent_home_with_path(
        "control::runtime::native_model::tests::settings_entry_tests::qoder::native_routing::native_routing_adapter_effects_restore_after_reopen_and_detect_drift",
        &path,
    ) {
        return;
    }
    for (kind, agent, mode) in [
        (AgentKindV1::Pi, "agent_pi_default", "pi_additional"),
        (
            AgentKindV1::DeepseekHarness,
            "agent_dsh_default",
            "dsh_additional",
        ),
        (
            AgentKindV1::OpenCode,
            "agent_opencode_default",
            "opencode_additional",
        ),
    ] {
        let mut fixture = if kind == AgentKindV1::OpenCode {
            QoderFixture::with_kind(true, kind)
        } else {
            QoderFixture::with_all_models()
        };
        fixture.context = fixture
            .runtime
            .adapter
            .settings_context_for_agent(agent)
            .unwrap();
        fixture.native_config = match kind {
            AgentKindV1::Pi => fixture.runtime.adapter.scanner.pi_user_models_target(),
            AgentKindV1::DeepseekHarness => {
                fixture.runtime.adapter.scanner.dsh_user_models_target()
            }
            _ => fixture.native_config.clone(),
        };
        let adapter = fixture
            .runtime
            .adapter
            .scanner
            .native_routing_adapter_target(kind)
            .unwrap();
        let baseline = fs::read(&fixture.native_config).unwrap();
        let mut spec = model_spec(&fixture);
        spec["model"]["settings"]["mode"] = json!(mode);
        apply(
            &fixture.service,
            &fixture.runtime,
            spec.clone(),
            "native-adapter-configure",
        );
        let bundled = hiroute_integrations::native_routing_adapter(kind)
            .unwrap()
            .as_bytes();
        assert_eq!(fs::read(&adapter).unwrap(), bundled);
        assert_eq!(model_status(&fixture)["state"], "configured");
        let fixture = reopen(fixture);
        assert_eq!(model_status(&fixture)["state"], "configured");
        let mut rotation = spec;
        rotation["access_token"] = json!({"intent":"regenerate"});
        apply(
            &fixture.service,
            &fixture.runtime,
            rotation,
            "native-adapter-rotate",
        );
        assert_eq!(fs::read(&adapter).unwrap(), bundled);
        private_file(&adapter, b"user changed this adapter\n");
        assert_eq!(model_status(&fixture)["state"], "drift");
        private_file(&adapter, bundled);
        apply(
            &fixture.service,
            &fixture.runtime,
            restore_spec(&fixture, &model_status(&fixture)),
            "native-adapter-restore",
        );
        assert_eq!(model_status(&fixture)["state"], "not_configured");
        assert!(!adapter.exists());
        assert_eq!(fs::read(&fixture.native_config).unwrap(), baseline);
    }
}
