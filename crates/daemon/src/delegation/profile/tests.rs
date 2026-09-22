use super::*;

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
        claude_context_window: Some(272_000),
        harness,
        adapter: Path::new("/trusted/adapter"),
        harness_binary: Path::new("/trusted/harness"),
        node_binary: None,
        private_root: private,
        session_root: session,
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
