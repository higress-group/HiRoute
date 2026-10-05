use super::*;

mod qoder;

#[cfg(unix)]
#[test]
fn nested_cli_shebang_resolves_the_selected_node_runtime() {
    // Avoid executing a newly written inode while other test threads fork with
    // its write descriptor inherited (ETXTBSY), without retrying the assertion.
    if crate::test_support::isolated_agent_home(
        "delegation::profile::tests::nested_cli_shebang_resolves_the_selected_node_runtime",
    ) {
        return;
    }
    use std::os::unix::fs::PermissionsExt;
    let fixture = private_fixture();
    let node = fixture.path().join("node");
    std::fs::write(&node, b"#!/bin/sh\necho selected-worker-node\n").unwrap();
    std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o700)).unwrap();
    let harness = fixture.path().join("native-cli");
    std::fs::write(&harness, b"#!/usr/bin/env node\n").unwrap();
    std::fs::set_permissions(&harness, std::fs::Permissions::from_mode(0o700)).unwrap();
    let path = worker_path(&harness, Some(&node), Some(&harness)).unwrap();
    assert_eq!(std::env::split_paths(&path).next().unwrap(), fixture.path());
    let result = std::process::Command::new(harness)
        .env_clear()
        .env("PATH", path)
        .output()
        .unwrap();
    assert!(result.status.success());
    assert_eq!(result.stdout, b"selected-worker-node\n");
}

#[cfg(unix)]
#[test]
fn isolated_worker_profiles_can_resolve_standard_tools() {
    for harness in [WorkerHarnessV1::ClaudeCode, WorkerHarnessV1::CodexCli] {
        let profile = request(harness, "private-token");
        assert_eq!(
            profile.native_selected_model_id(),
            "hiroute/1234567890abcdef"
        );
        assert!(profile.codex_initialization_root().is_none());
        let path = profile.env.get("PATH").expect("worker PATH");
        let entries: Vec<_> = std::env::split_paths(path.as_str()).collect();
        assert!(entries.iter().all(|entry| entry.is_absolute()));
        assert!(entries.contains(&std::path::PathBuf::from("/trusted")));
        assert_eq!(
            entries
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            entries.len()
        );
        let output = std::process::Command::new("/bin/sh")
            .env_clear()
            .envs(
                profile
                    .env
                    .iter()
                    .map(|(key, value)| (key.as_str(), value.as_str())),
            )
            .args(["-c", "test -n \"$PATH\" && command -v sh && command -v env"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{:?}", output);
    }
}

fn request(harness: WorkerHarnessV1, token: &str) -> CandidateWorkerProfile {
    request_with_alias(harness, token, "hiroute/1234567890abcdef")
}

fn request_with_alias(
    harness: WorkerHarnessV1,
    token: &str,
    alias: &str,
) -> CandidateWorkerProfile {
    request_with_scope(harness, token, alias, WorkerPermissionPolicyV1::ApproveAll)
}

fn request_with_scope(
    harness: WorkerHarnessV1,
    token: &str,
    alias: &str,
    permission_policy: WorkerPermissionPolicyV1,
) -> CandidateWorkerProfile {
    try_request_with_scope(harness, token, alias, permission_policy).unwrap()
}

fn try_request_with_scope(
    harness: WorkerHarnessV1,
    token: &str,
    alias: &str,
    permission_policy: WorkerPermissionPolicyV1,
) -> Result<CandidateWorkerProfile, DelegationErrorV1> {
    let fixture = private_fixture();
    let session = TaskSessionRoot::prepare(
        fixture.path(),
        &hiroute_domain::WorkspaceId::parse("workspace").unwrap(),
        "root",
        "task-a",
        harness,
        SessionRootUse::New,
    )
    .unwrap();
    try_request_at(
        harness,
        token,
        alias,
        permission_policy,
        &fixture.path().join("run"),
        &session,
    )
}

#[allow(clippy::too_many_arguments)]
fn try_request_at(
    harness: WorkerHarnessV1,
    token: &str,
    alias: &str,
    permission_policy: WorkerPermissionPolicyV1,
    private: &Path,
    session: &TaskSessionRoot,
) -> Result<CandidateWorkerProfile, DelegationErrorV1> {
    try_request_at_with_catalog(
        harness,
        token,
        alias,
        permission_policy,
        private,
        session,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn try_request_at_with_catalog(
    harness: WorkerHarnessV1,
    token: &str,
    alias: &str,
    permission_policy: WorkerPermissionPolicyV1,
    private: &Path,
    session: &TaskSessionRoot,
    codex_catalog: Option<&[u8]>,
) -> Result<CandidateWorkerProfile, DelegationErrorV1> {
    let context = NativeWorkerContext::isolated(private, session.path())?;
    try_request_in_context(
        harness,
        token,
        alias,
        permission_policy,
        private,
        session,
        codex_catalog,
        &context,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn try_request_in_context(
    harness: WorkerHarnessV1,
    token: &str,
    alias: &str,
    permission_policy: WorkerPermissionPolicyV1,
    private: &Path,
    session: &TaskSessionRoot,
    codex_catalog: Option<&[u8]>,
    context: &NativeWorkerContext,
    node_binary: Option<&Path>,
) -> Result<CandidateWorkerProfile, DelegationErrorV1> {
    let fixture_cli = session.path().join("selected-claude-fixture");
    let harness_binary = if harness == WorkerHarnessV1::ClaudeCode && context.is_borrowed() {
        std::fs::write(
            &fixture_cli,
            b"#!/bin/sh\n[ \"$1\" = --version ] || exit 23\nprintf '2.1.231 (Claude Code)\\n'\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fixture_cli, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        fixture_cli.as_path()
    } else {
        Path::new("/trusted/harness")
    };
    let permit = WorkspaceExecutionPermitV1 {
        permit_id: "p".into(),
        generation: 1,
        root_identity: "root".into(),
        access: WorkspaceAccessV1::TrustedNative,
        tools: vec![WorkerToolV1::Read, WorkerToolV1::Edit, WorkerToolV1::Shell],
        network: WorkerNetworkV1::Allowed,
        expires_at_ms: 100_000,
        max_run_ms: 10_000,
        max_concurrent: 2,
        revoked: false,
    };
    CandidateWorkerProfile::build(ProfileInput {
        context_window_tokens: Some(272_000),
        max_output_tokens: (harness == WorkerHarnessV1::QoderCli).then_some(4096),
        harness,
        adapter: match harness {
            WorkerHarnessV1::CodexCli | WorkerHarnessV1::ClaudeCode => {
                Some(Path::new("/trusted/adapter"))
            }
            WorkerHarnessV1::QoderCli | WorkerHarnessV1::Pi => None,
        },
        harness_binary,
        node_binary,
        private_root: private,
        session_root: session,
        native_context: context,
        workspace: Path::new("/work/a"),
        alias,
        codex_catalog,
        native_effort: Some("high"),
        gateway: "127.0.0.1:44123".parse().unwrap(),
        permit: &permit,
        execution: &WorkerExecutionIntentV1 {
            root_identity: "root".into(),
            access: WorkspaceAccessV1::TrustedNative,
            tools: vec![WorkerToolV1::Read, WorkerToolV1::Edit, WorkerToolV1::Shell],
            network: WorkerNetworkV1::Allowed,
            duration_ms: 10_000,
            delegation_depth: 1,
        },
        permission_policy,
        admitted_at_ms: 10,
        token: ProtectedSecret::new(token.as_bytes().to_vec()).unwrap(),
    })
}

#[test]
fn codex_worker_catalog_is_private_exact_and_reused_by_continue() {
    let fixture = private_fixture();
    let alias = "hiroute-fabuyanshou-codex-luna";
    let session = TaskSessionRoot::prepare(
        fixture.path(),
        &hiroute_domain::WorkspaceId::parse("workspace").unwrap(),
        "root",
        "task-a",
        WorkerHarnessV1::CodexCli,
        SessionRootUse::New,
    )
    .unwrap();
    let catalog = test_codex_catalog(alias);
    let first = try_request_at_with_catalog(
        WorkerHarnessV1::CodexCli,
        "run-secret",
        alias,
        WorkerPermissionPolicyV1::ApproveAll,
        &fixture.path().join("run-a"),
        &session,
        Some(&catalog),
    )
    .unwrap();
    let path = session.path().join("worker-model-catalog.json");
    assert_eq!(std::fs::read(&path).unwrap(), catalog);
    let config: Value = serde_json::from_str(first.env["CODEX_CONFIG"].as_str()).unwrap();
    assert_eq!(config["model_catalog_json"], path.to_str().unwrap());
    let startup = std::fs::read_to_string(session.path().join("config.toml")).unwrap();
    assert!(startup.contains(&format!(
        "model_catalog_json = {}",
        serde_json::to_string(path.to_str().unwrap()).unwrap()
    )));
    assert!(startup.contains("supports_websockets = false\n"));
    assert!(!first.env["CODEX_CONFIG"].contains("run-secret"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    try_request_at_with_catalog(
        WorkerHarnessV1::CodexCli,
        "next-secret",
        alias,
        WorkerPermissionPolicyV1::ApproveAll,
        &fixture.path().join("run-b"),
        &session,
        Some(&catalog),
    )
    .unwrap();
    let mut drifted: Value = serde_json::from_slice(&catalog).unwrap();
    drifted["models"][0]["description"] = json!("changed");
    let drifted = serde_json::to_vec(&drifted).unwrap();
    assert!(matches!(
        try_request_at_with_catalog(
            WorkerHarnessV1::CodexCli,
            "next-secret",
            alias,
            WorkerPermissionPolicyV1::ApproveAll,
            &fixture.path().join("run-c"),
            &session,
            Some(&drifted),
        ),
        Err(DelegationErrorV1::Conflict)
    ));
}

#[test]
fn codex_permission_policy_uses_only_the_proven_autonomous_native_mode() {
    let profile = request_with_scope(
        WorkerHarnessV1::CodexCli,
        "secret",
        "alias",
        WorkerPermissionPolicyV1::ApproveAll,
    );
    let config: Value = serde_json::from_str(profile.env["CODEX_CONFIG"].as_str()).unwrap();
    assert_eq!(config["approval_policy"], "never");
    assert_eq!(config["sandbox_mode"], "danger-full-access");
    assert_eq!(config["web_search"], "live");
    assert_eq!(config["features"]["shell_tool"], true);
    assert_eq!(
        profile.env["INITIAL_AGENT_MODE"].as_str(),
        "agent-full-access"
    );
    assert_eq!(profile.native_session_mode(), "agent-full-access");

    for policy in [
        WorkerPermissionPolicyV1::ApproveReads,
        WorkerPermissionPolicyV1::DenyAll,
    ] {
        assert!(matches!(
            try_request_with_scope(WorkerHarnessV1::CodexCli, "secret", "alias", policy),
            Err(DelegationErrorV1::CapabilityUnavailable)
        ));
    }
}

#[test]
fn managed_codex_profile_routes_only_through_run_env_without_embedding_secret_in_config() {
    let profile = request(WorkerHarnessV1::CodexCli, "run-secret-a");
    let config: Value = serde_json::from_str(profile.env["CODEX_CONFIG"].as_str()).unwrap();
    assert_eq!(config["approval_policy"], "never");
    assert_eq!(config["sandbox_mode"], "danger-full-access");
    assert_eq!(
        profile.env["INITIAL_AGENT_MODE"].as_str(),
        "agent-full-access"
    );
    assert_eq!(profile.native_session_mode(), "agent-full-access");
    assert_eq!(config["web_search"], "live");
    assert_eq!(config["model"], "hiroute/1234567890abcdef");
    assert_eq!(
        config["model_providers"]["hiroute"]["base_url"],
        "http://127.0.0.1:44123/v1"
    );
    assert_eq!(
        config["model_providers"]["hiroute"]["env_key"],
        "HIROUTE_RUN_TOKEN"
    );
    assert_eq!(
        config["model_providers"]["hiroute"]["supports_websockets"],
        false
    );
    assert!(!profile.env["CODEX_CONFIG"].contains("run-secret-a"));
    assert_eq!(profile.env["HIROUTE_RUN_TOKEN"].as_str(), "run-secret-a");
    assert_eq!(
        profile.env["CODEX_HOME"].as_str(),
        profile.session_root.to_str().unwrap()
    );
    assert!(!profile.env.contains_key("OPENAI_API_KEY"));
    assert!(!profile.env.contains_key("DEFAULT_AUTH_REQUEST"));
}

#[test]
fn managed_claude_profile_disables_ambient_settings_and_native_delegation() {
    let profile = request(WorkerHarnessV1::ClaudeCode, "run-secret-b");
    assert_eq!(profile.env["ANTHROPIC_AUTH_TOKEN"].as_str(), "run-secret-b");
    assert_eq!(
        profile.env["CLAUDE_CODE_AUTO_COMPACT_WINDOW"].as_str(),
        "272000"
    );
    assert_eq!(
        profile.env["CLAUDE_CODE_MAX_CONTEXT_TOKENS"].as_str(),
        "272000"
    );
    assert_eq!(
        profile.env["ANTHROPIC_BASE_URL"].as_str(),
        "http://127.0.0.1:44123"
    );
    assert_eq!(
        profile.env["CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC"].as_str(),
        "1"
    );
    assert!(!profile.env.contains_key("CLAUDE_CODE_OAUTH_TOKEN"));
    let options = &profile.session_meta["claudeCode"]["options"];
    assert_eq!(options["settingSources"], json!([]));
    assert_eq!(
        options["tools"],
        json!([
            "Read",
            "Glob",
            "Grep",
            "Edit",
            "Write",
            "Bash",
            "WebSearch",
            "WebFetch"
        ])
    );
    assert!(options.get("permissionMode").is_none());
    assert!(options.get("allowDangerouslySkipPermissions").is_none());
    assert_eq!(profile.native_session_mode(), "bypassPermissions");
    assert_eq!(options["mcpServers"], json!({}));
    assert!(
        options["disallowedTools"]
            .as_array()
            .unwrap()
            .contains(&json!("Agent"))
    );
}

#[test]
fn independent_profiles_replace_token_without_reusing_original_credentials() {
    for harness in [WorkerHarnessV1::CodexCli, WorkerHarnessV1::ClaudeCode] {
        let first = request(harness, "old-run-secret");
        let next = request(harness, "new-run-secret");
        assert_eq!(first.session_meta, next.session_meta);
        assert!(next.env.values().all(|v| !v.contains("old-run-secret")));
        assert!(next.env.values().any(|v| v.as_str() == "new-run-secret"));
    }
}

#[cfg(unix)]
#[test]
fn borrowed_codex_keeps_daily_config_and_routes_before_startup_with_task_owned_materials() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = private_fixture();
    let daily_home = fixture.path().join("user's home $(not-a-command)");
    let config_root = daily_home.join("custom-codex");
    std::fs::create_dir_all(&config_root).unwrap();
    let daily_config = b"model_provider = 'hiroute'\n[model_providers.hiroute]\nbase_url = 'https://daily.invalid'\n";
    std::fs::write(config_root.join("config.toml"), daily_config).unwrap();
    let context = NativeWorkerContext::borrowed(&daily_home, &config_root).unwrap();
    let session = TaskSessionRoot::prepare(
        fixture.path(),
        &hiroute_domain::WorkspaceId::parse("workspace").unwrap(),
        "root",
        "borrowed-task",
        WorkerHarnessV1::CodexCli,
        SessionRootUse::New,
    )
    .unwrap();
    let catalog = test_codex_catalog("frozen-alias");
    let first = try_request_in_context(
        WorkerHarnessV1::CodexCli,
        "first-run-secret",
        "frozen-alias",
        WorkerPermissionPolicyV1::ApproveAll,
        &fixture.path().join("first-run"),
        &session,
        Some(&catalog),
        &context,
        None,
    )
    .unwrap();
    let continued = try_request_in_context(
        WorkerHarnessV1::CodexCli,
        "fresh-run-secret",
        "frozen-alias",
        WorkerPermissionPolicyV1::ApproveAll,
        &fixture.path().join("next-run"),
        &session,
        Some(&catalog),
        &context,
        None,
    )
    .unwrap();
    assert_eq!(first.env["HOME"].as_str(), daily_home.to_str().unwrap());
    assert_eq!(
        first.codex_initialization_root(),
        Some(config_root.as_path())
    );
    assert_eq!(
        continued.codex_initialization_root(),
        Some(config_root.as_path())
    );
    assert_eq!(
        first.env["CODEX_HOME"].as_str(),
        config_root.to_str().unwrap()
    );
    assert_eq!(first.materials.directories, vec![PathBuf::from("tmp")]);
    assert_eq!(first.session_root, session.path());
    assert_eq!(
        std::fs::read(config_root.join("config.toml")).unwrap(),
        daily_config
    );
    assert!(!session.path().join("config.toml").exists());
    assert_eq!(
        std::fs::read(session.path().join("worker-model-catalog.json")).unwrap(),
        catalog
    );
    assert_eq!(first.env["MODEL_PROVIDER"], continued.env["MODEL_PROVIDER"]);
    assert_ne!(first.env["MODEL_PROVIDER"].as_str(), "hiroute");
    assert_ne!(first.env["CODEX_PATH"], continued.env["CODEX_PATH"]);
    assert_eq!(
        continued.env["HIROUTE_RUN_TOKEN"].as_str(),
        "fresh-run-secret"
    );
    assert!(!continued.env.contains_key("OPENAI_API_KEY"));
    assert!(!continued.env.contains_key("DEFAULT_AUTH_REQUEST"));
    let config: Value = serde_json::from_str(first.env["CODEX_CONFIG"].as_str()).unwrap();
    let provider = config["model_provider"].as_str().unwrap();
    assert_eq!(
        config["model_providers"][provider]["env_key"],
        "HIROUTE_RUN_TOKEN"
    );
    assert_eq!(first.materials.files.len(), 1);
    let launcher = &first.materials.files[0];
    assert!(launcher.executable);
    assert_eq!(
        first.env["CODEX_PATH"].as_str(),
        first
            .private_root
            .join(&launcher.relative_path)
            .to_str()
            .unwrap()
    );
    assert!(!String::from_utf8_lossy(&launcher.contents).contains("first-run-secret"));
    assert!(!first.env["CODEX_CONFIG"].contains("first-run-secret"));

    // Execute the actual rendered shell with a fake binary, checking argv before app-server.
    // The quoted path must remain a single executable and must not run command substitution.
    let native = daily_home.join("native's cli");
    std::fs::write(&native, b"#!/bin/sh\nprintf '%s\\0' \"$@\"\n").unwrap();
    std::fs::set_permissions(&native, std::fs::Permissions::from_mode(0o700)).unwrap();
    let script = super::codex::render_launcher(&native, &config).unwrap();
    let script_path = fixture.path().join("launcher");
    std::fs::write(&script_path, script).unwrap();
    let output = std::process::Command::new("/bin/sh")
        .arg(script_path)
        .arg("app-server")
        .env_clear()
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output.status);
    let args = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|value| !value.is_empty())
        .map(|value| std::str::from_utf8(value).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(args.last(), Some(&"app-server"));
    assert!(
        args[..args.len() - 1]
            .chunks_exact(2)
            .all(|pair| pair[0] == "-c")
    );
    for expected in [
        "model=\"frozen-alias\"".to_owned(),
        format!("model_provider=\"{provider}\""),
        format!("model_providers.{provider}.base_url=\"http://127.0.0.1:44123/v1\""),
        format!("model_providers.{provider}.env_key=\"HIROUTE_RUN_TOKEN\""),
        format!("model_providers.{provider}.requires_openai_auth=false"),
        format!("model_providers.{provider}.supports_websockets=false"),
        format!("model_catalog_json={}", config["model_catalog_json"]),
    ] {
        assert!(
            args.contains(&expected.as_str()),
            "missing managed startup override: {expected}"
        );
    }
}

#[cfg(unix)]
#[test]
fn borrowed_claude_enables_skills_without_expanding_restricted_permissions() {
    let fixture = private_fixture();
    let context =
        NativeWorkerContext::borrowed(Path::new("/instance-home"), Path::new("/native-claude"))
            .unwrap();
    let session = TaskSessionRoot::prepare(
        fixture.path(),
        &hiroute_domain::WorkspaceId::parse("workspace").unwrap(),
        "root",
        "borrowed-claude",
        WorkerHarnessV1::ClaudeCode,
        SessionRootUse::New,
    )
    .unwrap();
    assert!(matches!(
        try_request_in_context(
            WorkerHarnessV1::ClaudeCode,
            "run-only-secret",
            "frozen-alias",
            WorkerPermissionPolicyV1::ApproveAll,
            &fixture.path().join("run-no-node"),
            &session,
            None,
            &context,
            None,
        ),
        Err(DelegationErrorV1::CapabilityUnavailable)
    ));
    for policy in [
        WorkerPermissionPolicyV1::ApproveAll,
        WorkerPermissionPolicyV1::ApproveReads,
        WorkerPermissionPolicyV1::DenyAll,
    ] {
        let profile = try_request_in_context(
            WorkerHarnessV1::ClaudeCode,
            "run-only-secret",
            "frozen-alias",
            policy,
            &fixture.path().join("run"),
            &session,
            None,
            &context,
            Some(Path::new("/trusted/selected-node")),
        )
        .unwrap();
        assert_eq!(profile.env["HOME"].as_str(), "/instance-home");
        assert!(profile.codex_initialization_root().is_none());
        assert_eq!(profile.env["CLAUDE_CONFIG_DIR"].as_str(), "/native-claude");
        assert_eq!(
            profile.env["CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST"].as_str(),
            "1"
        );
        assert_eq!(
            profile.env["ANTHROPIC_CUSTOM_MODEL_OPTION"].as_str(),
            "frozen-alias"
        );
        assert_eq!(profile.env["ANTHROPIC_MODEL"].as_str(), "frozen-alias");
        assert_eq!(
            profile.env["ANTHROPIC_AUTH_TOKEN"].as_str(),
            "run-only-secret"
        );
        assert!(!profile.env.contains_key("ANTHROPIC_API_KEY"));
        assert!(!profile.env.contains_key("CLAUDE_CODE_OAUTH_TOKEN"));
        assert_eq!(profile.env["NO_PROXY"].as_str(), "127.0.0.1");
        assert_eq!(profile.env["no_proxy"].as_str(), "127.0.0.1");
        assert_eq!(profile.materials.directories, vec![PathBuf::from("tmp")]);
        assert_eq!(profile.executable, Path::new("/trusted/selected-node"));
        assert_eq!(profile.materials.files.len(), 1);
        let bootstrap = &profile.materials.files[0];
        assert!(!bootstrap.executable);
        assert!(!String::from_utf8_lossy(&bootstrap.contents).contains("run-only-secret"));
        assert_eq!(
            profile.args,
            vec![
                profile.private_root.join(&bootstrap.relative_path),
                PathBuf::from("/trusted/adapter")
            ]
        );
        assert!(!profile.env.contains_key("NODE_OPTIONS"));
        let options = &profile.session_meta["claudeCode"]["options"];
        assert_eq!(
            options["settingSources"],
            json!(["user", "project", "local"])
        );
        assert_eq!(options["model"], "frozen-alias");
        assert_eq!(options["strictMcpConfig"], true);
        assert_eq!(options["mcpServers"], json!({}));
        assert_eq!(options["settings"]["disableAllHooks"], true);
        assert_eq!(options["settings"]["apiKeyHelper"], "");
        assert_eq!(
            options["settings"]["env"],
            json!({"NO_PROXY": "127.0.0.1", "no_proxy": "127.0.0.1"})
        );
        assert!(options.get("plugins").is_none());
        assert!(options.get("hooks").is_none());
        assert!(!options.to_string().contains("run-only-secret"));
        let tools = options["tools"].as_array().unwrap();
        assert_eq!(
            tools.contains(&json!("Skill")),
            policy == WorkerPermissionPolicyV1::ApproveAll
        );
        if policy == WorkerPermissionPolicyV1::ApproveReads {
            assert_eq!(tools, &vec![json!("Read"), json!("Glob"), json!("Grep")]);
        } else if policy == WorkerPermissionPolicyV1::DenyAll {
            assert!(tools.is_empty());
        }
    }
}

#[test]
fn version_or_policy_label_is_not_a_confinement_proof() {
    let profile = request(WorkerHarnessV1::CodexCli, "run-secret");
    let unknown = WorkerPlatformCapabilities::default();
    assert_eq!(
        profile.require_platform(&unknown),
        Err(DelegationErrorV1::CapabilityUnavailable)
    );
    let partial = WorkerPlatformCapabilities {
        can_start: true,
        can_stop: true,
    };
    assert!(profile.require_platform(&partial).is_ok());
}

#[test]
fn published_readable_and_legacy_model_names_are_preserved_exactly() {
    for alias in ["reviewer-strong", "hiroute/1234567890abcdef"] {
        let codex = request_with_alias(WorkerHarnessV1::CodexCli, "secret", alias);
        let config: Value = serde_json::from_str(codex.env["CODEX_CONFIG"].as_str()).unwrap();
        assert_eq!(config["model"], alias);
        let claude = request_with_alias(WorkerHarnessV1::ClaudeCode, "secret", alias);
        assert_eq!(claude.env["ANTHROPIC_MODEL"].as_str(), alias);
        assert_eq!(claude.session_meta["claudeCode"]["options"]["model"], alias);
    }
}

#[test]
fn requested_readonly_scope_is_not_widened_to_the_broader_standing_permit() {
    let profile = request_with_scope(
        WorkerHarnessV1::ClaudeCode,
        "secret",
        "alias",
        WorkerPermissionPolicyV1::ApproveReads,
    );
    assert_eq!(profile.access(), WorkspaceAccessV1::ReadOnly);
    assert_eq!(profile.network(), WorkerNetworkV1::GatewayOnly);
    assert_eq!(profile.tools(), &[WorkerToolV1::Read]);
    assert_eq!(
        profile.session_meta["claudeCode"]["options"]["tools"],
        json!(["Read", "Glob", "Grep"])
    );
    assert!(
        profile.session_meta["claudeCode"]["options"]
            .get("permissionMode")
            .is_none()
    );
    assert!(
        profile.session_meta["claudeCode"]["options"]
            .get("allowDangerouslySkipPermissions")
            .is_none()
    );
    assert_eq!(profile.native_session_mode(), "default");
}

#[test]
fn explicit_native_mode_runs_without_universal_confinement_flags() {
    for harness in [WorkerHarnessV1::CodexCli, WorkerHarnessV1::ClaudeCode] {
        let profile = request_with_scope(
            harness,
            "secret",
            "alias",
            WorkerPermissionPolicyV1::ApproveAll,
        );
        assert!(
            profile
                .require_platform(&WorkerPlatformCapabilities {
                    can_start: true,
                    can_stop: true
                })
                .is_ok()
        );
        assert!(profile.permission_limitations().contains("no same-user"));
        assert!(profile.allows_permission_once(&json!({"toolCall":{"kind":"execute"}})));
        assert!(profile.allows_permission_once(&json!({"toolCall":{"kind":"edit"}})));
        assert!(profile.allows_permission_once(&json!({"toolCall":{"kind":"other"}})));
        if harness == WorkerHarnessV1::CodexCli {
            let config: Value = serde_json::from_str(profile.env["CODEX_CONFIG"].as_str()).unwrap();
            assert_eq!(config["approval_policy"], "never");
            assert_eq!(config["sandbox_mode"], "danger-full-access");
            assert_eq!(
                profile.env["INITIAL_AGENT_MODE"].as_str(),
                "agent-full-access"
            );
            assert_eq!(profile.native_session_mode(), "agent-full-access");
            assert_eq!(config["sandbox_workspace_write"]["network_access"], true);
        } else {
            assert_eq!(profile.native_session_mode(), "bypassPermissions");
            assert!(
                profile.session_meta["claudeCode"]["options"]
                    .get("permissionMode")
                    .is_none()
            );
            assert!(
                profile.session_meta["claudeCode"]["options"]["tools"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("Bash"))
            );
        }
    }
    let strict = request_with_scope(
        WorkerHarnessV1::ClaudeCode,
        "secret",
        "alias",
        WorkerPermissionPolicyV1::ApproveReads,
    );
    assert!(strict.require_native_mode().is_ok());
    assert!(strict.allows_permission_once(&json!({"toolCall":{"kind":"read"}})));
    assert!(!strict.allows_permission_once(&json!({"toolCall":{"kind":"edit"}})));
    let denied = request_with_scope(
        WorkerHarnessV1::ClaudeCode,
        "secret",
        "alias",
        WorkerPermissionPolicyV1::DenyAll,
    );
    assert!(!denied.allows_permission_once(&json!({"toolCall":{"kind":"read"}})));
}

#[path = "materials_tests.rs"]
mod materials_tests;

fn private_fixture() -> tempfile::TempDir {
    let fixture = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(fixture.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    fixture
}

#[path = "capability_tests.rs"]
mod capability_tests;
