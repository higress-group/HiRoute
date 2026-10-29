//! Real protected adapter effects and restarted transaction/status readers.
use super::models::{model_spec, model_status, restore_spec};
use super::*;

#[test]
fn selected_agent_discovery_preserves_identity_without_neighbor_probes() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::qoder::native_routing::selected_agent_discovery_preserves_identity_without_neighbor_probes",
    ) {
        return;
    }
    let fixture = QoderFixture::with_all_models();
    let context = fixture
        .runtime
        .adapter
        .settings_context_for_agent("agent_dsh_default")
        .unwrap();
    // A discover-all implementation would execute this unrelated installed CLI.
    let neighbor = fixture._root.path().join("missing-claude");
    let marker = fixture._root.path().join("unrelated-probe");
    private_file(
        &neighbor,
        format!(
            "#!/bin/sh\nprintf hit > '{}'\nprintf '2.1.231\\n'\n",
            marker.display()
        )
        .as_bytes(),
    );
    fs::set_permissions(&neighbor, fs::Permissions::from_mode(0o700)).unwrap();
    let reply =
        fixture
            .service
            .dispatch_wire(request("ScanAgents", json!({"context_id":context}), None));
    assert!(reply.error.is_none(), "{reply:?}");
    let data = reply.data.unwrap();
    assert_eq!(data["agents"].as_array().unwrap().len(), 1);
    assert_eq!(data["agents"][0]["agent_id"], "agent_dsh_default");
    assert_eq!(data["agents"][0]["context_id"], context);
    assert!(!marker.exists(), "selected read probed a neighboring Agent");
    for input in [
        json!({"context_id":"agent-context/dsh/unknown"}),
        json!({"context_id":""}),
        json!({"context_id":context,"native_target":{"kind":"hermes","config_root":"/unrelated-profile"}}),
    ] {
        assert!(
            fixture
                .service
                .dispatch_wire(request("ScanAgents", input, None))
                .error
                .is_some()
        );
    }
    assert!(!marker.exists());
    let full = fixture
        .service
        .dispatch_wire(request("ScanAgents", json!({}), None));
    assert!(
        full.error.is_none(),
        "released empty request still scans all Agents"
    );
    assert!(
        marker.exists(),
        "negative control proves the unrelated executable is discoverable"
    );
}

#[test]
fn dsh_save_checks_composition_once_per_capture_and_again_at_admission() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::qoder::native_routing::dsh_save_checks_composition_once_per_capture_and_again_at_admission",
    ) {
        return;
    }
    let mut fixture = QoderFixture::with_all_models();
    fixture.context = fixture
        .runtime
        .adapter
        .settings_context_for_agent("agent_dsh_default")
        .unwrap();
    fixture.native_config = fixture.runtime.adapter.scanner.dsh_user_models_target();
    let executable = fixture
        .runtime
        .adapter
        .scanner
        .dsh_executable_target()
        .unwrap();
    let calls = fixture._root.path().join("dsh-probe-calls");
    private_file(&executable, format!("#!/bin/sh\nprintf '%s\\n' \"$1\" >> '{}'\nif [ \"$1\" = --version ]; then echo 'DSH 0.2.0'; else printf '%s\\n' '- id: llm-pi-ai'; fi\n", calls.display()).as_bytes());
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let mut spec = model_spec(&fixture);
    spec["model"]["settings"]["mode"] = json!("dsh_additional");
    apply(
        &fixture.service,
        &fixture.runtime,
        spec.clone(),
        "dsh-probe-count",
    );
    assert_eq!(model_status(&fixture)["state"], "configured");
    // Preview and Apply admission each own one independent proof.
    // Apply does not build and clone a complete plan before entering its writer.
    // The second file snapshot neither starts another engine nor skips drift checks.
    assert_eq!(
        fs::read_to_string(&calls).unwrap(),
        "--profile\n--profile\n"
    );
    let fresh = fixture.service.dispatch_wire(request(
        "PreviewAgentConnectionChange",
        json!({"spec":spec}),
        None,
    ));
    assert!(fresh.error.is_none(), "{fresh:?}");
    let fresh = fresh.data.unwrap();
    let before = fs::read(&fixture.native_config).unwrap();
    private_file(
        &executable,
        b"#!/bin/sh\nprintf '%s\\n' '- id: wrong-module'\n",
    );
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let rejected = fixture.service.dispatch_wire(request(
        "ApplyAgentConnectionChange", json!({"spec":spec,
            "accept_digest":fresh["accept_digest"], "dependency_digest":fresh["dependency_digest"],
            "expected_revisions":fresh["expected_revisions"], "idempotency_key":"changed-after-preview"}), None,
    ));
    assert!(
        rejected.error.is_some(),
        "changed native composition must be rejected at admission"
    );
    assert!(rejected.operation.is_none());
    assert_eq!(fs::read(&fixture.native_config).unwrap(), before);
    let spec: api::AgentSettingsSpecV2 = serde_json::from_value(spec).unwrap();
    assert!(
        fixture
            .runtime
            .adapter
            .capture_settings_facts(&spec)
            .is_err(),
        "a previous call cannot authorize a changed installation"
    );
}

#[test]
fn changed_dsh_mapping_compiles_each_grant_once() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::qoder::native_routing::changed_dsh_mapping_compiles_each_grant_once",
    ) {
        return;
    }
    let mut fixture = QoderFixture::with_all_models();
    fixture.context = fixture
        .runtime
        .adapter
        .settings_context_for_agent("agent_dsh_default")
        .unwrap();
    fixture.native_config = fixture.runtime.adapter.scanner.dsh_user_models_target();
    let mut spec = model_spec(&fixture);
    spec["model"]["settings"]["mode"] = json!("dsh_additional");
    let plans = fixture
        .runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .active_publication(&WorkspaceId::default())
        .unwrap()
        .unwrap()
        .verify()
        .unwrap()
        .published_agent_plans()
        .unwrap()
        .into_iter()
        .filter(|p| {
            p.active
                && p.supported_ingress
                    .contains(&AgentIngressProtocolV1::Responses)
        })
        .map(|p| p.agent_plan_id)
        .collect::<Vec<_>>();
    assert!(
        plans.len() > 1,
        "exercise a real changed route set, not a no-op save"
    );
    let mut last = None;
    for (key, selection) in [
        ("initial-mapping", vec![plans[0].clone()]),
        ("added-mapping", plans),
    ] {
        spec["model"]["settings"]["allowed_plan_ids"] = json!(selection);
        let builds = || {
            crate::control::runtime::publication::SETTINGS_RECORD_BUILDS.with(std::cell::Cell::get)
        };
        let before = builds();
        last = Some(apply(&fixture.service, &fixture.runtime, spec.clone(), key));
        assert_eq!(model_status(&fixture)["state"], "configured");
        assert_eq!(
            builds() - before,
            1,
            "{key}: observing and applying the same authenticated grant must not recompile its publication"
        );
    }
    let id = OperationId::parse(last.unwrap()["operation_id"].as_str().unwrap()).unwrap();
    let mut operation = fixture
        .runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .load_operation(&id)
        .unwrap()
        .unwrap();
    // Exercise pre-journal computation with authenticated settings input. Execution
    // still requires the current journal, independently of this pure candidate.
    operation
        .step_mut(OperationStepKind::CompilePublication)
        .effects
        .clear();
    let intent = operation
        .plan
        .external()
        .iter()
        .find(|i| i.kind() == OwnedEffectKind::Publication)
        .unwrap()
        .clone();
    let expected = fixture
        .runtime
        .adapter
        .publication_record_from_operation(&operation, &intent)
        .unwrap();
    let mut changed = operation.clone();
    changed.accepted_digest = CanonicalDigest::of_bytes(b"different-confirmation");
    assert!(
        fixture
            .runtime
            .adapter
            .publication_record_from_operation(&changed, &intent)
            .is_err()
    );
    changed = operation.clone();
    changed.workspace_id = WorkspaceId::parse("workspace/other").unwrap();
    assert!(
        fixture
            .runtime
            .adapter
            .publication_record_from_operation(&changed, &intent)
            .is_err()
    );
    // Detached equal input must authenticate again instead of inheriting a pointer proof.
    changed = operation.clone();
    changed.plan = Arc::new((*operation.plan).clone());
    let builds =
        || crate::control::runtime::publication::SETTINGS_RECORD_BUILDS.with(std::cell::Cell::get);
    let before = builds();
    assert_eq!(
        fixture
            .runtime
            .adapter
            .publication_record_from_operation(&changed, &intent)
            .unwrap(),
        expected
    );
    assert_eq!(builds() - before, 1);
    let before = fs::read(&fixture.native_config).unwrap();
    let reopened = reopen(fixture);
    assert_eq!(model_status(&reopened)["state"], "configured");
    assert_eq!(fs::read(&reopened.native_config).unwrap(), before);
}

pub(super) fn reopen(fixture: QoderFixture) -> QoderFixture {
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
        if kind == AgentKindV1::Pi {
            assert!(adapter.ends_with("extensions/hiroute-routing.js"));
        }
        if kind == AgentKindV1::OpenCode {
            assert!(adapter.ends_with("plugins/hiroute-routing.js"));
        }
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
        if kind == AgentKindV1::DeepseekHarness {
            let mut edited = b"# unrelated user edit before token rotation\n".to_vec();
            edited.extend_from_slice(&fs::read(&fixture.native_config).unwrap());
            private_file(&fixture.native_config, &edited);
        }
        let mut rotation = spec.clone();
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
        let restored = fs::read(&fixture.native_config).unwrap();
        if kind == AgentKindV1::DeepseekHarness {
            assert!(
                !String::from_utf8_lossy(&restored).contains("hiroute-routing"),
                "restoration must remove the owned plugin insert before deleting its file"
            );
            assert!(restored.starts_with(b"# unrelated user edit before token rotation\n"));
        } else {
            assert_eq!(restored, baseline);
        }
        // Borrowing identical native resources must not turn into file ownership.
        private_file(&adapter, bundled);
        apply(
            &fixture.service,
            &fixture.runtime,
            spec,
            "native-adapter-borrow",
        );
        apply(
            &fixture.service,
            &fixture.runtime,
            restore_spec(&fixture, &model_status(&fixture)),
            "native-adapter-release-reference",
        );
        assert_eq!(fs::read(&adapter).unwrap(), bundled);
        fs::remove_file(adapter).unwrap();
    }
}
