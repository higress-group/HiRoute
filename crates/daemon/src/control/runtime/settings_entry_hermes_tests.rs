//! Public wire settings with real stores/files; native SDK acceptance is a separate gate.
use super::*;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

fn private_file(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

const ORIGINAL: &[u8] = b"# user config\nmodel:\n  default: native/model\n  provider: native\n  context_length: 64000\n  api_key: synthetic-native-key\nproviders:\n  native:\n    api: https://native.invalid/v1\n    transport: chat_completions\nauxiliary:\n  vision:\n    model: native/vision\nchannels:\n  discord: unchanged\n";

fn isolate(test: &str) -> bool {
    let source = crate::test_support::private_tempdir();
    let interpreter = source.path().join("venv/bin/python");
    private_file(
        &interpreter,
        b"structural interpreter locator, never executed",
    );
    private_file(
        &source.path().join("hermes"),
        format!("#!/bin/sh\n# '{}'\nexit 97\n", interpreter.display()).as_bytes(),
    );
    for resource in [
        "openai/resources/responses/responses.py",
        "anthropic/resources/messages/messages.py",
    ] {
        private_file(
            &source
                .path()
                .join("venv/lib/python3.13/site-packages")
                .join(resource),
            b"def create(*, extra_headers: Headers): pass\n# structural fixture, never imported\n",
        );
    }
    fs::set_permissions(
        source.path().join("hermes"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    private_file(
        &source.path().join("hermes_cli/plugins.py"),
        b"def register_middleware(): pass\n# plugins\n",
    );
    private_file(
        &source.path().join("hermes_cli/middleware.py"),
        b"def apply_llm_request_middleware(): pass\n",
    );
    private_file(&source.path().join("agent/conversation_loop.py"), b"apply_llm_request_middleware(request, session_id=agent.session_id, model=agent.model, provider=agent.provider, base_url=agent.base_url, api_mode=agent.api_mode)\n");
    private_file(
        &source.path().join("tools/skills_tool.py"),
        b"def _skills_dir(): return get_hermes_home() / \"skills\"\n# structural fixture, never executed\n",
    );
    private_file(
        &source.path().join("tools/terminal_tool.py"),
        b"def terminal_tool(command): pass\n# terminal interface fixture, never executed\n",
    );
    private_file(
        &source.path().join("hermes_cli/config.py"),
        b"def load_config_readonly(): pass\n",
    );
    private_file(
        &source.path().join("pyproject.toml"),
        b"[project]\nversion = \"0.19.1\"\n",
    );
    let path =
        std::env::join_paths([source.path(), Path::new("/usr/bin"), Path::new("/bin")]).unwrap();
    crate::test_support::isolated_agent_home_with_path(test, &path)
}

struct Fixture {
    root: tempfile::TempDir,
    profile: PathBuf,
    runtime: ProductionControlRuntime,
    service: LocalControlDaemon,
    scanner: FilesystemAgentScannerV1,
    context: String,
}
impl Fixture {
    fn new(models: bool) -> Self {
        Self::with_publication(models, golden_publication_plans_only())
    }
    fn with_publication(models: bool, publication: GatewayPublicationV1) -> Self {
        let root = crate::test_support::private_tempdir();
        let profile = root.path().join("chosen-profile");
        private_file(&profile.join("config.yaml"), ORIGINAL);
        private_file(
            &profile.join("sessions/preserved"),
            b"unrelated native session",
        );
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        let mut layout = AgentFilesystemLayoutV1::from_process(&home, root.path());
        layout.codex_executable = root.path().join("missing-codex");
        layout.claude_executable = root.path().join("missing-claude");
        layout.qoder_executable = root.path().join("missing-qoder");
        layout.pi_executable = root.path().join("missing-pi");
        layout.dsh_executable = root.path().join("missing-dsh");
        let registry = serde_json::from_slice(include_bytes!(
            "../../../../../assets/connector-registry/current/registry-seed.json"
        ))
        .unwrap();
        let models_data: hiroute_domain::ReleaseModelDataBundleV2 = serde_json::from_slice(
            include_bytes!("../../../../../assets/release-facts/current/bundle/model-data.json"),
        )
        .unwrap();
        let scanner = FilesystemAgentScannerV1::new(
            layout,
            ClaudeRegistrationIndexV1::from_verified_model_data(&registry, &models_data.data)
                .unwrap(),
        );
        let storage = root.path().join("storage");
        let runtime = ProductionControlRuntime::prepare_for_role_all_with_scanner(
            &storage,
            crate::release_catalog::fixture_catalog(),
            None,
            scanner.clone(),
        )
        .unwrap();
        let runtime = if models {
            configure_runtime_with_initial(&storage, runtime, publication)
        } else {
            runtime
        };
        runtime.adapter.reconcile_startup_and_open().unwrap();
        let service = LocalControlDaemon::new(ApplicationService::new(runtime.application_ports()));
        let context = hiroute_integrations::hermes_context_id(profile.to_str().unwrap());
        Self {
            root,
            profile,
            runtime,
            service,
            scanner,
            context,
        }
    }
    fn base(&self) -> serde_json::Value {
        json!({"schema_version":{"major":2,"minor":0}, "context_id":self.context,
            "native_target":{"kind":"hermes","config_root":self.profile}, "model":{"intent":"keep"}})
    }
    fn model_spec(&self, index: usize) -> serde_json::Value {
        let publication = self
            .runtime
            .adapter
            .stores_lock()
            .unwrap()
            .control()
            .active_publication(&WorkspaceId::default())
            .unwrap()
            .unwrap()
            .verify()
            .unwrap();
        let plans: Vec<_> = publication
            .published_agent_plans()
            .unwrap()
            .into_iter()
            .filter(|p| {
                p.active
                    && p.supported_ingress
                        .contains(&AgentIngressProtocolV1::Responses)
            })
            .collect();
        let mut spec = self.base();
        spec["model"] = json!({"intent":"configure","settings":{"mode":"hermes_default","plan_id":plans[index].agent_plan_id,"protocol":"responses"}});
        spec
    }
    fn status(&self) -> serde_json::Value {
        let result = self.service.dispatch_wire(request(
            "GetAgentConnectionStatus",
            json!({"schema_version":{"major":2,"minor":0},"context_id":self.context}),
            None,
        ));
        assert!(result.error.is_none(), "{result:?}");
        result.data.unwrap()
    }
    fn restore(&self) -> serde_json::Value {
        json!({"schema_version":{"major":2,"minor":0},"context_id":self.context,"model":{"intent":"restore","restore_point_ref":self.status()["restore_point_ref"]}})
    }
    fn reopen(self) -> Self {
        self.reopen_with_publication(true)
    }
    fn reopen_with_publication(self, publication: bool) -> Self {
        let Self {
            root,
            profile,
            runtime,
            service,
            scanner,
            context,
        } = self;
        drop(service);
        drop(runtime);
        let storage = root.path().join("storage");
        let runtime = ProductionControlRuntime::prepare_for_role_all_with_scanner(
            &storage,
            crate::release_catalog::fixture_catalog(),
            None,
            scanner.clone(),
        )
        .unwrap();
        let runtime = if publication {
            configure_runtime_with_initial(&storage, runtime, golden_publication_plans_only())
        } else {
            runtime
        };
        runtime.adapter.reconcile_startup_and_open().unwrap();
        let service = LocalControlDaemon::new(ApplicationService::new(runtime.application_ports()));
        Self {
            root,
            profile,
            runtime,
            service,
            scanner,
            context,
        }
    }
}

#[test]
fn management_only_is_independent_and_recoverable_without_model_publication() {
    if isolate(
        "control::runtime::native_model::tests::settings_entry_tests::hermes::management_only_is_independent_and_recoverable_without_model_publication",
    ) {
        return;
    }
    let fixture = Fixture::new(false);
    let mut spec = fixture.base();
    spec["management_skill"] = json!({"intent":"configure","settings":{}});
    // Malformed unrelated model config does not become a management-Skill dependency.
    private_file(
        &fixture.profile.join("config.yaml"),
        b"model: [invalid-for-model]\n",
    );
    apply(
        &fixture.service,
        &fixture.runtime,
        spec,
        "hermes-skill-only",
    );
    assert_eq!(fixture.status()["state"], "not_configured");
    assert_eq!(fixture.status()["management_skill"]["state"], "configured");
    assert!(fixture.status().get("collaboration").is_none());
    assert_eq!(
        fs::read(fixture.profile.join("skills/hiroute-management/SKILL.md")).unwrap(),
        include_bytes!("../../../../../assets/skills/hiroute-management/SKILL.md")
    );
    let stores = fixture.runtime.adapter.stores_lock().unwrap();
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
                &format!("agent-connection/{}", fixture.context)
            )
            .unwrap()
            .is_none()
    );
    drop(stores);
    let spec = json!({"schema_version":{"major":2,"minor":0},"context_id":fixture.context,
        "management_skill":{"intent":"restore","restore_point_ref":fixture.status()["management_skill"]["restore_point_ref"]}});
    let preview = fixture.service.dispatch_wire(request(
        "PreviewAgentConnectionRestore",
        json!({"spec":spec}),
        None,
    ));
    assert!(preview.error.is_none(), "{preview:?}");
    let preview = preview.data.unwrap();
    let result = fixture.service.dispatch_wire(request("ApplyAgentConnectionRestore", json!({"spec":spec,"accept_digest":preview["accept_digest"],"dependency_digest":preview["dependency_digest"],"expected_revisions":preview["expected_revisions"],"idempotency_key":"hermes-skill-remove"}), None));
    assert!(result.error.is_none(), "{result:?}");
    assert_eq!(result.data.unwrap()["state"], "succeeded");
    assert!(!fixture.profile.join("skills/hiroute-management").exists());
    assert_eq!(
        fs::read(fixture.profile.join("config.yaml")).unwrap(),
        b"model: [invalid-for-model]\n"
    );
}

// The frozen catalog predates Chat admission. Add a declared Chat renderer to
// each fixture candidate without broadening the released Plan advice projection.
fn chat_publication() -> GatewayPublicationV1 {
    let mut publication = golden_publication_plans_only().into_current().unwrap();
    for plan in &mut publication.plans {
        let mut body = plan.body.as_ref().clone();
        for group in &mut body.materialized.attempt_owned.groups {
            for candidate in &mut group.candidates {
                let mut chat = candidate
                    .protocol_profiles
                    .iter()
                    .find(|profile| {
                        profile.ingress_protocol == hiroute_domain::UpstreamProtocol::Responses
                    })
                    .unwrap()
                    .clone();
                chat.path_id = "fixture-chat-to-responses".into();
                chat.ingress_protocol = hiroute_domain::UpstreamProtocol::ChatCompletions;
                candidate.protocol_profiles.push(chat);
                candidate.protocol_profile_digest =
                    CanonicalDigest::of(&candidate.protocol_profiles).unwrap();
            }
        }
        body.materialized_route_digest = body.materialized.route_digest().unwrap();
        *plan = hiroute_domain::CompiledAgentPlanV1::seal_current(body).unwrap();
    }
    publication.plan_heads.clear();
    publication.validate_current_contract().unwrap();
    publication
}

#[test]
fn old_chat_can_configure_without_reasoning_replay_but_required_interfaces_still_block() {
    if isolate(
        "control::runtime::native_model::tests::settings_entry_tests::hermes::old_chat_can_configure_without_reasoning_replay_but_required_interfaces_still_block",
    ) {
        return;
    }
    let fixture = Fixture::with_publication(true, chat_publication());
    let source = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .next()
        .unwrap();
    let sdk = source
        .join("venv/lib/python3.13/site-packages/openai/resources/chat/completions/completions.py");
    let middleware = source.join("hermes_cli/middleware.py");
    let mut spec = fixture.model_spec(0);
    spec["model"]["settings"]["protocol"] = json!("chat_completions");
    let preview = || {
        fixture.service.dispatch_wire(request(
            "PreviewAgentConnectionChange",
            json!({"spec":spec}),
            None,
        ))
    };
    // Source declares 0.19.1 and has no replay helpers. SDK/header and plugin
    // incompatibilities remain genuine pre-write blockers independently of that version.
    for (sdk_source, middleware_source) in [
        (
            "def create(): pass\n",
            "def apply_llm_request_middleware(): pass\n",
        ),
        (
            "def create(*, extra_headers: Headers): pass\n",
            "# incompatible middleware\n",
        ),
    ] {
        private_file(&sdk, sdk_source.as_bytes());
        private_file(&middleware, middleware_source.as_bytes());
        let result = preview();
        assert!(
            result
                .error
                .as_ref()
                .is_some_and(|error| error.code == api::ErrorCode::CapabilityDenied)
                || result
                    .data
                    .as_ref()
                    .is_some_and(|data| data["applicable"] == false),
            "{result:?}"
        );
        assert_eq!(
            fs::read(fixture.profile.join("config.yaml")).unwrap(),
            ORIGINAL
        );
        assert!(!fixture.profile.join("plugins/hiroute").exists());
    }
    private_file(&sdk, b"def create(*, extra_headers: Headers): pass\n");
    private_file(&middleware, b"def apply_llm_request_middleware(): pass\n");
    let result = preview();
    assert!(result.error.is_none(), "{result:?}");
    assert_eq!(result.data.unwrap()["applicable"], true);
    apply(
        &fixture.service,
        &fixture.runtime,
        spec,
        "hermes-old-chat-connect",
    );
    let status = fixture.status();
    assert_eq!(status["state"], "configured");
    assert_eq!(status["current_selection"]["protocol"], "chat_completions");
    assert_eq!(status["native_adapter"]["resources_installed"], true);
    assert!(
        fs::read_to_string(fixture.profile.join("config.yaml"))
            .unwrap()
            .contains("chat_completions")
    );
    let restore = fixture.restore();
    apply(
        &fixture.service,
        &fixture.runtime,
        restore,
        "hermes-old-chat-restore",
    );
    assert_eq!(
        fs::read(fixture.profile.join("config.yaml")).unwrap(),
        ORIGINAL
    );
    assert!(!fixture.profile.join("plugins/hiroute").exists());
    assert_eq!(
        fs::read(fixture.profile.join("sessions/preserved")).unwrap(),
        b"unrelated native session"
    );
}

#[test]
fn model_resources_rehydrate_after_restart_and_restore_without_touching_skill() {
    if isolate(
        "control::runtime::native_model::tests::settings_entry_tests::hermes::model_resources_rehydrate_after_restart_and_restore_without_touching_skill",
    ) {
        return;
    }
    let fixture = Fixture::new(true);
    let mut spec = fixture.model_spec(0);
    spec["management_skill"] = json!({"intent":"configure","settings":{}});
    apply(&fixture.service, &fixture.runtime, spec, "hermes-main");
    assert_eq!(
        fs::read(fixture.profile.join("plugins/hiroute/__init__.py")).unwrap(),
        hiroute_integrations::HERMES_PLUGIN_MODULE.as_bytes()
    );
    assert_eq!(fixture.status()["model_verified"], false);
    let fixture = fixture.reopen();
    assert_eq!(fixture.status()["state"], "configured");
    assert_eq!(
        fixture.status()["native_target"]["config_root"],
        json!(fixture.profile)
    );
    assert_eq!(
        fixture.status()["native_adapter"]["runtime_loading"],
        "unobserved"
    );
    let listed = fixture
        .service
        .dispatch_wire(request("ListAgents", json!({}), None))
        .data
        .unwrap();
    assert!(
        listed["agents"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["context_id"] == fixture.context)
    );
    let scan = fixture
        .service
        .dispatch_wire(request("ScanAgents", json!({}), None))
        .data
        .unwrap();
    assert!(
        !scan["agents"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["agent_id"] == "agent_hermes_default")
    );
    let selected = fixture.service.dispatch_wire(request(
        "ScanAgents",
        json!({"context_id":fixture.context}),
        None,
    ));
    assert!(selected.error.is_none(), "{selected:?}");
    let selected = selected.data.unwrap();
    assert_eq!(selected["agents"].as_array().unwrap().len(), 1);
    assert_eq!(selected["agents"][0]["context_id"], fixture.context);
    assert_eq!(
        selected["agents"][0]["native_target"]["config_root"],
        fixture.profile.to_str().unwrap()
    );
    let spec = fixture.restore();
    assert!(spec.get("native_target").is_none());
    let preview = fixture.service.dispatch_wire(request(
        "PreviewAgentConnectionRestore",
        json!({"spec":spec}),
        None,
    ));
    assert!(preview.error.is_none(), "{preview:?}");
    assert_eq!(
        preview.data.unwrap()["spec"]["native_target"]["config_root"],
        fixture.profile.to_str().unwrap()
    );
    // Released callers can submit their original intent with the confirmed digest.
    apply(
        &fixture.service,
        &fixture.runtime,
        spec,
        "hermes-restore-after-restart",
    );
    assert_eq!(
        fs::read(fixture.profile.join("config.yaml")).unwrap(),
        ORIGINAL
    );
    assert!(!fixture.profile.join("plugins/hiroute").exists());
    assert_eq!(fixture.status()["management_skill"]["state"], "configured");
    assert_eq!(
        fs::read(fixture.profile.join("sessions/preserved")).unwrap(),
        b"unrelated native session"
    );
}

#[test]
fn borrowed_plugin_files_survive_restore_and_foreign_collision_blocks_before_write() {
    if isolate(
        "control::runtime::native_model::tests::settings_entry_tests::hermes::borrowed_plugin_files_survive_restore_and_foreign_collision_blocks_before_write",
    ) {
        return;
    }
    let fixture = Fixture::new(true);
    let module = fixture.profile.join("plugins/hiroute/__init__.py");
    let manifest = fixture.profile.join("plugins/hiroute/plugin.yaml");
    private_file(&module, b"foreign plugin");
    let preview = fixture.service.dispatch_wire(request(
        "PreviewAgentConnectionChange",
        json!({"spec":fixture.model_spec(0)}),
        None,
    ));
    assert_eq!(preview.data.unwrap()["applicable"], false);
    assert_eq!(
        fs::read(fixture.profile.join("config.yaml")).unwrap(),
        ORIGINAL
    );
    private_file(
        &module,
        hiroute_integrations::HERMES_PLUGIN_MODULE.as_bytes(),
    );
    private_file(
        &manifest,
        hiroute_integrations::HERMES_PLUGIN_MANIFEST.as_bytes(),
    );
    let inode = fs::metadata(&module).unwrap().ino();
    apply(
        &fixture.service,
        &fixture.runtime,
        fixture.model_spec(0),
        "hermes-borrow",
    );
    assert_eq!(fs::metadata(&module).unwrap().ino(), inode);
    assert_eq!(fs::metadata(&module).unwrap().mode() & 0o777, 0o600);
    apply(
        &fixture.service,
        &fixture.runtime,
        fixture.restore(),
        "hermes-borrow-restore",
    );
    assert!(module.exists() && manifest.exists());
    assert_eq!(
        fs::read(&module).unwrap(),
        hiroute_integrations::HERMES_PLUGIN_MODULE.as_bytes()
    );
}

#[test]
fn reconfigure_then_conditional_restore_preserves_foreign_edits_and_rejects_target_rebind() {
    if isolate(
        "control::runtime::native_model::tests::settings_entry_tests::hermes::reconfigure_then_conditional_restore_preserves_foreign_edits_and_rejects_target_rebind",
    ) {
        return;
    }
    let fixture = Fixture::new(true);
    apply(
        &fixture.service,
        &fixture.runtime,
        fixture.model_spec(0),
        "hermes-plan-0",
    );
    apply(
        &fixture.service,
        &fixture.runtime,
        fixture.model_spec(1),
        "hermes-plan-1",
    );
    let mut changed = fs::read(fixture.profile.join("config.yaml")).unwrap();
    changed.extend_from_slice(b"user_note: retained\n");
    private_file(&fixture.profile.join("config.yaml"), &changed);
    let different = fixture.root.path().join("another-profile");
    fs::create_dir(&different).unwrap();
    let mut spec = fixture.model_spec(0);
    spec["native_target"]["config_root"] = json!(different);
    let rejected = fixture.service.dispatch_wire(request(
        "PreviewAgentConnectionChange",
        json!({"spec":spec}),
        None,
    ));
    assert!(rejected.error.is_some());
    apply(
        &fixture.service,
        &fixture.runtime,
        fixture.restore(),
        "hermes-conditional-restore",
    );
    let restored =
        String::from_utf8(fs::read(fixture.profile.join("config.yaml")).unwrap()).unwrap();
    assert!(
        restored.contains("\"default\": \"native/model\""),
        "{restored}"
    );
    assert!(restored.contains("user_note: retained"), "{restored}");
    assert!(restored.contains("  discord: unchanged"), "{restored}");
}

#[test]
fn independent_skills_without_model_plugin_survive_restart_and_restore_separately() {
    if isolate(
        "control::runtime::native_model::tests::settings_entry_tests::hermes::independent_skills_without_model_plugin_survive_restart_and_restore_separately",
    ) {
        return;
    }
    let source = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .next()
        .unwrap();
    fs::remove_file(source.join("hermes_cli/plugins.py")).unwrap();
    fs::remove_file(source.join("hermes_cli/middleware.py")).unwrap();
    fs::remove_dir_all(source.join("venv/lib")).unwrap();
    let support = hiroute_integrations::hermes_support_facts();
    assert!(support.skills && support.shell);
    assert!(!support.middleware && !support.responses_sdk && !support.messages_sdk);
    let fixture = Fixture::new(false);
    private_file(
        &fixture.profile.join("config.yaml"),
        b"model: [unused malformed model]\n",
    );
    let mut spec = fixture.base();
    spec["management_skill"] = json!({"intent":"configure","settings":{}});
    spec["collaboration"] =
        json!({"intent":"configure","settings":{"trigger_mode":"delegate_by_default"}});
    apply(
        &fixture.service,
        &fixture.runtime,
        spec,
        "hermes-both-skills",
    );
    let management = fixture.profile.join("skills/hiroute-management/SKILL.md");
    let collaboration = fixture
        .profile
        .join("skills/hiroute-collaboration/SKILL.md");
    assert!(management.is_file() && collaboration.is_file());
    assert!(!fixture.profile.join("plugins").exists());
    assert!(
        fs::read_to_string(&collaboration)
            .unwrap()
            .contains("answer-only")
    );
    let fixture = fixture.reopen_with_publication(false);
    let response=fixture.service.dispatch_wire(request("GetAgentConnectionStatus",json!({"schema_version":{"major":2,"minor":0},"context_id":fixture.context,"include_skills":true}),None));
    assert!(response.error.is_none(), "{response:?}");
    let status = response.data.unwrap();
    assert_eq!(status["collaboration"]["state"], "configured");
    assert_eq!(status["management_skill"]["state"], "configured");
    assert_ne!(
        status["collaboration"]["restore_point_ref"],
        status["management_skill"]["restore_point_ref"]
    );
    let mut spec = fixture.base();
    spec["collaboration"] = json!({"intent":"restore","restore_point_ref":status["collaboration"]["restore_point_ref"]});
    apply(
        &fixture.service,
        &fixture.runtime,
        spec,
        "hermes-restore-collaboration",
    );
    assert!(!collaboration.exists());
    assert!(management.is_file());
    let mut spec = fixture.base();
    spec["management_skill"] = json!({"intent":"restore","restore_point_ref":status["management_skill"]["restore_point_ref"]});
    apply(
        &fixture.service,
        &fixture.runtime,
        spec,
        "hermes-restore-management-last",
    );
    assert!(!management.exists());
    assert!(!fixture.profile.join("plugins").exists());
    assert_eq!(
        fs::read(fixture.profile.join("config.yaml")).unwrap(),
        b"model: [unused malformed model]\n"
    );
}

#[path = "settings_effective_tests.rs"]
mod effective;
