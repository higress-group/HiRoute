//! Public V2 settings -> coordinator -> real stores and Skill files. The explicit capability
//! fixture does not prove native Qoder loading or real delegation; product acceptance does.
use super::*;
use crate::control::runtime::settings_facts::SettingsAgentClass;
use hiroute_integrations::QoderCollaborationProbeTarget;
use std::path::{Path, PathBuf};

struct QoderFixture {
    _root: tempfile::TempDir,
    runtime: ProductionControlRuntime,
    service: LocalControlDaemon,
    context: String,
    native_config: PathBuf,
    skill: PathBuf,
    untouched: Vec<(PathBuf, Vec<u8>)>,
}

impl QoderFixture {
    fn new() -> Self {
        Self::with_models(false)
    }

    fn with_models(models: bool) -> Self {
        Self::with_kind(models, AgentKindV1::Qoder)
    }

    fn with_kind(models: bool, kind: AgentKindV1) -> Self {
        let pi = kind == AgentKindV1::Pi;
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        let config_root = home.join(if pi { "selected-pi" } else { "selected-qoder" });
        private_file(
            &config_root.join("settings.json"),
            b"native settings are opaque to HiRoute\n",
        );
        let executable = root.path().join("qoder-fixture");
        private_file(&executable, b"#!/bin/sh\nexit 97\n");
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let mut layout = AgentFilesystemLayoutV1::from_process(&home, root.path());
        layout.codex_executable = root.path().join("missing-codex");
        layout.claude_executable = root.path().join("missing-claude");
        if pi {
            let package = root.path().join("selected-pi-package");
            private_file(&package.join("package.json"), br#"{"name":"@earendil-works/pi-coding-agent","version":"1.0.2","bin":{"pi":"dist/bundle/cli.js"}}"#);
            private_file(&package.join("dist/index.js"), b"test SDK location only");
            private_file(&package.join("dist/bundle/cli.js"), b"#!/bin/sh\nexit 97\n");
            fs::set_permissions(
                package.join("dist/bundle/cli.js"),
                fs::Permissions::from_mode(0o700),
            )
            .unwrap();
            layout.pi_executable = package.join("dist/bundle/cli.js");
            layout.pi_config_root = config_root.clone();
            private_file(
                &config_root.join("settings.json"),
                br#"{"defaultProvider":"native","defaultModel":"default","theme":"dark"}"#,
            );
        } else {
            layout.pi_executable = root.path().join("missing-pi");
            layout.qoder_executable = executable;
        }
        layout.qoder_home = home.clone();
        if !pi {
            layout.qoder_config_root = config_root.clone();
        } else {
            layout.qoder_executable = root.path().join("missing-qoder");
        }
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
            ClaudeRegistrationIndexV1::from_verified_model_data(&registry, &model_data.data)
                .unwrap(),
        );
        let native_config = if pi {
            scanner.pi_user_models_target()
        } else {
            scanner.qoder_user_config_target()
        };
        let skill = if pi {
            scanner.pi_user_skill_target()
        } else {
            scanner.qoder_user_skill_target()
        };
        // Native executables/workspace are not storage contents. Production deliberately
        // refuses to initialize a store over an unrelated pre-existing file tree.
        let storage = root.path().join("storage");
        let runtime = ProductionControlRuntime::prepare_for_role_all_with_scanner(
            &storage,
            crate::release_catalog::fixture_catalog(),
            None,
            scanner,
        )
        .unwrap();
        let runtime = if models {
            configure_runtime_with_initial(&storage, runtime, golden_publication_plans_only())
        } else {
            runtime
        };
        if models {
            private_file(
                &native_config,
                if pi {
                    br#"{"providers":{"native":{"user":"preserved"}},"unknown":{"keep":true}}"#
                } else {
                    br#"{"model":{"name":"native/default"},"providers":{"native":{"user":"preserved"}},"unknown":{"keep":true}}"#
                },
            );
        }
        runtime.adapter.reconcile_startup_and_open().unwrap();
        let context = runtime
            .adapter
            .settings_context_for_agent(if pi {
                "agent_pi_default"
            } else {
                "agent_qoder_default"
            })
            .unwrap();
        let service = LocalControlDaemon::new(ApplicationService::new(
            runtime.application_ports().with_agent_connection(Arc::new(
                FixtureFacts::collaboration(runtime.adapter.clone()),
            )),
        ));
        let mut untouched = vec![(native_config.clone(), fs::read(&native_config).unwrap())];
        for path in [
            config_root.join("auth.json"),
            config_root.join("skills/personal/SKILL.md"),
            home.join(".qoder/settings.json"),
            home.join(".hiroute/credential-artifacts/qoder.sealed"),
            home.join(".hiroute/credential-artifacts/codex.sealed"),
            home.join(".hiroute/credential-artifacts/claude-code.sealed"),
        ] {
            let bytes = b"synthetic user-owned sentinel; never a HiRoute credential\n".to_vec();
            private_file(&path, &bytes);
            untouched.push((path, bytes));
        }
        Self {
            _root: root,
            runtime,
            service,
            context,
            native_config,
            skill,
            untouched,
        }
    }

    fn configure(&self, mode: &str) -> serde_json::Value {
        json!({"schema_version":{"major":2,"minor":0},"context_id":self.context,
            "collaboration":{"intent":"configure","settings":{"trigger_mode":mode}}})
    }

    fn status(&self) -> serde_json::Value {
        let response = self.service.dispatch_wire(request(
            "GetAgentConnectionStatus",
            json!({"schema_version":{"major":2,"minor":0},"context_id":self.context}),
            None,
        ));
        assert!(response.error.is_none(), "{response:?}");
        let status = response.data.unwrap();
        assert_eq!(status["schema"], "hiroute.agent-model-settings-status/v2");
        assert_eq!(status["context_id"], self.context);
        assert_eq!(status["state"], "not_configured");
        assert_eq!(status["model_verified"], false);
        assert_eq!(status["current_selection"], serde_json::Value::Null);
        assert!(
            status["live_check_targets"].is_null() || status["live_check_targets"] == json!([])
        );
        status
    }

    fn restore(&self) -> serde_json::Value {
        json!({"schema_version":{"major":2,"minor":0},"context_id":self.context,
            "collaboration":{"intent":"restore","restore_point_ref":self.status()["collaboration"]["restore_point_ref"]}})
    }

    fn assert_no_model_effects(&self) {
        for (path, bytes) in &self.untouched {
            assert_eq!(
                fs::read(path).unwrap(),
                *bytes,
                "changed {}",
                path.display()
            );
        }
        let stores = self.runtime.adapter.stores_lock().unwrap();
        assert!(
            stores
                .control()
                .active_publication(&WorkspaceId::default())
                .unwrap()
                .is_none()
        );
        assert!(
            stores
                .secrets()
                .inspect_agent_access_grant(
                    WorkspaceId::DEFAULT,
                    &format!("agent-connection/{}", self.context)
                )
                .unwrap()
                .is_none()
        );
        assert_eq!(
            stores
                .secrets()
                .agent_access_grant_generation(
                    WorkspaceId::DEFAULT,
                    &format!("agent-connection/{}", self.context)
                )
                .unwrap(),
            0
        );
        for operation in stores
            .control()
            .succeeded_agent_operations_for_kinds(
                &WorkspaceId::default(),
                &["ApplyAgentConnectionChange", "ApplyAgentConnectionRestore"],
            )
            .unwrap()
        {
            assert!(operation.plan.agent_access_grants().is_empty());
            let target = SettingsAgentClass::Qoder.skill_target().unwrap();
            assert!(
                operation
                    .plan
                    .external()
                    .iter()
                    .all(|effect| effect.target() == target)
            );
        }
    }
}

fn private_file(path: &Path, bytes: &[u8]) {
    let parent = path.parent().unwrap();
    fs::create_dir_all(parent).unwrap();
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn qoder_collaboration_configure_trigger_replay_and_restore_preserve_native_context() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::qoder::qoder_collaboration_configure_trigger_replay_and_restore_preserve_native_context",
    ) {
        return;
    }
    let fixture = QoderFixture::new();
    assert!(!SettingsAgentClass::Qoder.is_codex());
    assert_ne!(
        fixture.context,
        fixture
            .runtime
            .adapter
            .settings_context(SettingsAgentClass::Codex)
    );
    assert!(
        fixture
            .native_config
            .ends_with("selected-qoder/settings.json")
    );
    let initial: api::AgentSettingsSpecV2 =
        serde_json::from_value(fixture.configure("explicit")).unwrap();
    let facts = fixture
        .runtime
        .adapter
        .capture_settings_facts(&initial)
        .unwrap();
    assert!(facts.model_file.is_none());
    assert!(facts.facts.model.is_none());
    assert!(matches!(
        fixture
            .runtime
            .adapter
            .qoder_collaboration_probe_target()
            .unwrap(),
        QoderCollaborationProbeTarget::PreinstallCapability
    ));
    let ordinary =
        LocalControlDaemon::new(ApplicationService::new(fixture.runtime.application_ports()));
    let blocked = ordinary.dispatch_wire(request(
        "PreviewAgentConnectionChange",
        json!({"spec":initial}),
        None,
    ));
    assert!(blocked.error.is_none(), "{blocked:?}");
    let blocked = blocked.data.unwrap();
    assert_eq!(blocked["applicable"], false);
    assert!(
        blocked["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|blocker| blocker["reason"] == "capability_unavailable")
    );

    apply(
        &fixture.service,
        &fixture.runtime,
        fixture.configure("delegate_by_default"),
        "qoder-enable",
    );
    assert_eq!(
        fixture.status()["collaboration"]["current_selection"]["trigger_mode"],
        "delegate_by_default"
    );
    let first = fs::read(&fixture.skill).unwrap();
    assert!(
        matches!(fixture.runtime.adapter.qoder_collaboration_probe_target().unwrap(),
        QoderCollaborationProbeTarget::InstalledUserSkill { expected_content }
            if expected_content == CanonicalDigest::of_bytes(&first))
    );
    apply(
        &fixture.service,
        &fixture.runtime,
        fixture.configure("explicit"),
        "qoder-explicit",
    );
    let second = fs::read(&fixture.skill).unwrap();
    assert_ne!(first, second);
    assert_eq!(
        fixture.status()["collaboration"]["current_selection"]["trigger_mode"],
        "explicit"
    );
    fixture.assert_no_model_effects();
    apply(
        &fixture.service,
        &fixture.runtime,
        fixture.restore(),
        "qoder-disable",
    );
    assert_eq!(fixture.status()["collaboration"]["state"], "restored");
    assert!(!fixture.skill.exists());
    fixture.assert_no_model_effects();
}

#[test]
fn qoder_restore_only_rejects_change_entry_without_mutation_and_restore_remains_readable() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::qoder::qoder_restore_only_rejects_change_entry_without_mutation_and_restore_remains_readable",
    ) {
        return;
    }
    let fixture = QoderFixture::new();
    apply(
        &fixture.service,
        &fixture.runtime,
        fixture.configure("explicit"),
        "qoder-restore-entry-enable",
    );
    let before_skill = fs::read(&fixture.skill).unwrap();
    let before_status = fixture.status();
    let before_revisions = fixture
        .runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .current_revisions(&WorkspaceId::default())
        .unwrap();
    let spec = fixture.restore();
    let wrong_preview = fixture.service.dispatch_wire(request(
        "PreviewAgentConnectionChange",
        json!({"spec":spec}),
        None,
    ));
    assert_eq!(
        wrong_preview.error.unwrap().code,
        api::ErrorCode::InvalidArguments
    );
    assert!(wrong_preview.operation.is_none());

    // A genuine Restore confirmation is still invalid on the Change descriptor. Reject
    // before admission rather than writing a succeeded journal row the strict reader rejects.
    let preview = fixture.service.dispatch_wire(request(
        "PreviewAgentConnectionRestore",
        json!({"spec":spec}),
        None,
    ));
    assert!(preview.error.is_none(), "{preview:?}");
    let preview = preview.data.unwrap();
    assert_eq!(preview["applicable"], true);
    let key = "qoder-restore-entry";
    let wrong_apply = fixture.service.dispatch_wire(request(
        "ApplyAgentConnectionChange",
        json!({"spec":spec,"accept_digest":preview["accept_digest"],
            "dependency_digest":preview["dependency_digest"],"expected_revisions":preview["expected_revisions"],
            "idempotency_key":key}),
        None,
    ));
    assert_eq!(
        wrong_apply.error.unwrap().code,
        api::ErrorCode::InvalidArguments
    );
    assert!(wrong_apply.operation.is_none());
    {
        let stores = fixture.runtime.adapter.stores_lock().unwrap();
        let control = stores.control();
        assert_eq!(
            control.current_revisions(&WorkspaceId::default()).unwrap(),
            before_revisions
        );
        assert!(control.writer_claim_operation().unwrap().is_none());
        for kind in ["ApplyAgentConnectionChange", "ApplyAgentConnectionRestore"] {
            let scope =
                hiroute_domain::IdempotencyScopeV1::new("interactive-user", kind, key).unwrap();
            assert!(
                control
                    .operation_for_idempotency(&WorkspaceId::default(), &scope)
                    .unwrap()
                    .is_none()
            );
        }
    }
    assert_eq!(fs::read(&fixture.skill).unwrap(), before_skill);
    assert_eq!(fixture.status(), before_status);

    // The same idempotency key remains usable on the correct descriptor. The shared apply
    // helper replays the exact confirmation; status exercises the strict succeeded reader.
    let restored = apply(&fixture.service, &fixture.runtime, spec, key);
    assert_eq!(fixture.status()["collaboration"]["state"], "restored");
    assert!(!fixture.skill.exists());
    let operation_id = OperationId::parse(restored["operation_id"].as_str().unwrap()).unwrap();
    let restored = fixture
        .runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .load_operation(&operation_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        restored.idempotency.operation_kind,
        "ApplyAgentConnectionRestore"
    );
    fixture.assert_no_model_effects();
}

#[test]
fn qoder_cross_client_model_and_unowned_restore_intents_are_rejected_before_any_operation() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::qoder::qoder_cross_client_model_and_unowned_restore_intents_are_rejected_before_any_operation",
    ) {
        return;
    }
    let fixture = QoderFixture::new();
    let model = json!({"intent":"configure","settings":{"mode":"codex_default",
        "native_model_mode":"hiroute_only","fixed_models":[],"allowed_plan_ids":["plan-qoder-forbidden"],
        "default_selection":{"kind":"plan","plan_id":"plan-qoder-forbidden"}}});
    for (intent, token) in [
        (model.clone(), json!({"intent":"keep"})),
        (model.clone(), json!({"intent":"regenerate"})),
        (
            model,
            json!({"intent":"set","input_slot":"agent-access-token-candidate/forbidden"}),
        ),
        (
            json!({"intent":"restore","restore_point_ref":"restore-point/forbidden"}),
            json!({"intent":"keep"}),
        ),
    ] {
        let mut spec = fixture.configure("explicit");
        spec["model"] = intent;
        spec["access_token"] = token;
        let restore = spec["model"]["intent"] == "restore";
        if restore {
            spec["collaboration"] = json!({"intent":"keep"});
        }
        let command = if restore {
            "PreviewAgentConnectionRestore"
        } else {
            "PreviewAgentConnectionChange"
        };
        let rejected = fixture
            .service
            .dispatch_wire(request(command, json!({"spec":spec}), None));
        let expected = if restore {
            api::ErrorCode::ResourceNotFound
        } else {
            api::ErrorCode::InvalidArguments
        };
        assert_eq!(rejected.error.unwrap().code, expected);
        assert!(rejected.operation.is_none());
        // Even a caller carrying a real collaboration preview cannot smuggle a model/token
        // intent past the backend confirmation boundary. Depending on the supplied intent,
        // its changed dependencies or the invalid model selection can reject first.
        let preview = fixture
            .service
            .dispatch_wire(request(
                "PreviewAgentConnectionChange",
                json!({"spec":fixture.configure("explicit")}),
                None,
            ))
            .data
            .unwrap();
        let rejected = fixture.service.dispatch_wire(request(
            if restore { "ApplyAgentConnectionRestore" } else { "ApplyAgentConnectionChange" },
            json!({"spec":spec,"accept_digest":preview["accept_digest"],
                "dependency_digest":preview["dependency_digest"],"expected_revisions":preview["expected_revisions"],
                "idempotency_key":"qoder-forbidden-model"}), None));
        assert!(matches!(
            rejected.error.unwrap().code,
            api::ErrorCode::InvalidArguments
                | api::ErrorCode::ChangePreviewStale
                | api::ErrorCode::ResourceNotFound
        ));
        assert!(rejected.operation.is_none());
    }
    assert!(!fixture.skill.exists());
    fixture.assert_no_model_effects();
}

#[test]
fn qoder_owned_skill_drift_rejects_stale_apply_and_keeps_installed_verification_phase() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::qoder::qoder_owned_skill_drift_rejects_stale_apply_and_keeps_installed_verification_phase",
    ) {
        return;
    }
    let fixture = QoderFixture::new();
    apply(
        &fixture.service,
        &fixture.runtime,
        fixture.configure("explicit"),
        "qoder-drift-enable",
    );
    let restore = fixture.restore();
    let preview = fixture
        .service
        .dispatch_wire(request(
            "PreviewAgentConnectionRestore",
            json!({"spec":restore}),
            None,
        ))
        .data
        .unwrap();
    assert_eq!(preview["applicable"], true);
    let edited = b"user changed this installed skill\n";
    private_file(&fixture.skill, edited);
    let edited_status = fixture.status();
    assert_eq!(edited_status["collaboration"]["state"], "drift");
    assert!(edited_status["collaboration"]["current_selection"].is_null());
    assert_eq!(
        edited_status["collaboration"]["restore_point_ref"],
        restore["collaboration"]["restore_point_ref"]
    );
    assert!(
        matches!(fixture.runtime.adapter.qoder_collaboration_probe_target().unwrap(),
        QoderCollaborationProbeTarget::InstalledUserSkill { expected_content }
            if expected_content != CanonicalDigest::of_bytes(edited))
    );
    let stale = fixture.service.dispatch_wire(request("ApplyAgentConnectionRestore", json!({
        "spec":restore,"accept_digest":preview["accept_digest"],"dependency_digest":preview["dependency_digest"],
        "expected_revisions":preview["expected_revisions"],"idempotency_key":"qoder-stale-restore"}), None));
    assert!(stale.error.is_some(), "{stale:?}");
    let fresh = fixture
        .service
        .dispatch_wire(request(
            "PreviewAgentConnectionRestore",
            json!({"spec":restore}),
            None,
        ))
        .data
        .unwrap();
    assert_eq!(fresh["applicable"], false);
    assert!(
        fresh["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|blocker| blocker["reason"] == "skill_file_conflict")
    );
    assert_eq!(fs::read(&fixture.skill).unwrap(), edited);
    fs::remove_file(&fixture.skill).unwrap();
    let missing_status = fixture.status();
    assert_eq!(missing_status["collaboration"]["state"], "drift");
    assert!(missing_status["collaboration"]["current_selection"].is_null());
    assert!(matches!(
        fixture
            .runtime
            .adapter
            .qoder_collaboration_probe_target()
            .unwrap(),
        QoderCollaborationProbeTarget::InstalledUserSkill { .. }
    ));
    fixture.assert_no_model_effects();
}

#[test]
fn qoder_borrowed_skill_restore_releases_only_the_reference_after_user_edits() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::native_model::tests::settings_entry_tests::qoder::qoder_borrowed_skill_restore_releases_only_the_reference_after_user_edits",
    ) {
        return;
    }
    let fixture = QoderFixture::new();
    let spec: api::AgentSettingsSpecV2 =
        serde_json::from_value(fixture.configure("explicit")).unwrap();
    let facts = fixture
        .runtime
        .adapter
        .capture_settings_facts(&spec)
        .unwrap();
    private_file(&fixture.skill, facts.skill_file.template.content.as_bytes());
    apply(
        &fixture.service,
        &fixture.runtime,
        fixture.configure("explicit"),
        "qoder-borrow",
    );
    let restore = fixture.restore();
    let record = fixture
        .runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .skill_installation(
            &WorkspaceId::default(),
            SettingsAgentClass::Qoder.skill_root_ref(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        record.file_ownership,
        CollaborationSkillFileOwnership::BorrowedIdentical
    );
    let edited = b"user keeps their independent skill\n";
    private_file(&fixture.skill, edited);
    apply(
        &fixture.service,
        &fixture.runtime,
        restore,
        "qoder-release-borrowed",
    );
    assert_eq!(fixture.status()["collaboration"]["state"], "restored");
    assert_eq!(fs::read(&fixture.skill).unwrap(), edited);
    let record = fixture
        .runtime
        .adapter
        .stores_lock()
        .unwrap()
        .control()
        .skill_installation(
            &WorkspaceId::default(),
            SettingsAgentClass::Qoder.skill_root_ref(),
        )
        .unwrap()
        .unwrap();
    assert!(!record.contexts.contains(&fixture.context));
    fixture.assert_no_model_effects();
}

#[path = "settings_entry_qoder_model_tests.rs"]
mod models;
