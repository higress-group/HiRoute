use std::fs;
use std::path::Path;

use hiroute_domain::{ConnectorRegistryBundleV1, ReleaseModelDataBundleV2};

use super::*;
use crate::agents::{
    AGENT_AUTH_PRECEDENCE_CONFLICT_V1, CLAUDE_CODE_VERIFIED_VERSION_2_1_231_V1,
    CLAUDE_CODE_VERIFIED_VERSION_V1, MANAGED_LAUNCH_ENV_SANITIZED_V1,
};

#[path = "filesystem_tests/claude_routing.rs"]
mod claude_routing;

fn write_executable(path: &Path, version: &str) {
    fs::write(path, format!("#!/bin/sh\nprintf '%s\\n' '{version}'\n")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
}

fn write_secret_settings(path: &Path, value: serde_json::Value) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path.parent().unwrap(), fs::Permissions::from_mode(0o700)).unwrap();
    }
    fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }
}

fn registry() -> ClaudeRegistrationIndexV1 {
    let registry: ConnectorRegistryBundleV1 = serde_json::from_slice(include_bytes!(
        "../../../../assets/connector-registry/current/registry-seed.json"
    ))
    .unwrap();
    let model_data: ReleaseModelDataBundleV2 = serde_json::from_slice(include_bytes!(
        "../../../../assets/release-facts/current/bundle/model-data.json"
    ))
    .unwrap();
    ClaudeRegistrationIndexV1::from_verified_model_data(&registry, &model_data.data).unwrap()
}

fn layout(root: &Path) -> AgentFilesystemLayoutV1 {
    layout_with_claude_version(root, CLAUDE_CODE_VERIFIED_VERSION_V1)
}

fn layout_with_claude_version(root: &Path, claude_version: &str) -> AgentFilesystemLayoutV1 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // tempfile 3.27 defaults directories to 0777 before umask. Keep the configuration
        // target private so executable-directory permissions are the only variable in these
        // discovery fixtures.
        fs::set_permissions(root, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap();
    let home = root.join("home");
    fs::create_dir(&home).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let codex = bin.join("codex");
    let claude = bin.join("claude");
    write_executable(&codex, "codex-cli diagnostic-build");
    write_executable(&claude, &format!("{claude_version} (Claude Code)"));
    AgentFilesystemLayoutV1 {
        codex_executable: codex,
        codex_desktop_executable: None,
        claude_executable: claude,
        codex_user_config: home.join(".codex/config.toml"),
        claude_launch_settings: None,
        claude_project_settings: vec![root.join("project/.claude/settings.json")],
        claude_user_settings: home.join(".claude/settings.json"),
        claude_managed_settings: Vec::new(),
        process_environment: BTreeMap::new(),
        process_environment_presence: BTreeSet::new(),
    }
}

#[test]
fn filesystem_scanner_exact_versions_registers_claude_and_never_serializes_token() {
    let directory = tempfile::tempdir().unwrap();
    let layout = layout(directory.path());
    write_secret_settings(
        &layout.claude_user_settings,
        json!({"env": {
            "ANTHROPIC_BASE_URL": "https://open.bigmodel.cn/api/anthropic",
            "ANTHROPIC_MODEL": "claude-opus-5",
            "ANTHROPIC_DEFAULT_OPUS_MODEL": "glm-5.3[1m]",
            "ANTHROPIC_AUTH_TOKEN": "secret-do-not-serialize"
        }}),
    );
    let scanner = FilesystemAgentScannerV1::new(layout, registry());
    let results = scanner.scan();
    assert_eq!(results.len(), 2);
    let claude = results
        .iter()
        .find(|result| outcome_agent_id(&result.outcome) == "agent_claude_default")
        .unwrap();
    assert_eq!(
        claude
            .claude_configuration
            .as_ref()
            .unwrap()
            .connection_option_id,
        "zhipu.coding-plan.cn.v1"
    );
    assert_eq!(
        claude
            .claude_configuration
            .as_ref()
            .unwrap()
            .provider_model_alias_hints["default_opus_model"],
        "glm-5.3[1m]"
    );
    let encoded = serde_json::to_string(&results).unwrap();
    assert!(!encoded.contains("secret-do-not-serialize"));
    assert!(!encoded.contains(directory.path().to_string_lossy().as_ref()));
    let secret = scanner
        .read_discovered_secret(claude.discovered_credential.as_ref().unwrap())
        .unwrap();
    assert_eq!(secret.expose(), b"secret-do-not-serialize");
}

#[test]
fn group_writable_codex_desktop_engine_is_located_without_cli_substitution() {
    let directory = tempfile::tempdir().unwrap();
    let mut layout = layout(directory.path());
    let marker = directory.path().join("codex-was-executed");
    let applications = directory.path().join("Applications");
    fs::create_dir(&applications).unwrap();
    let desktop_engine = applications.join("codex-desktop-engine");
    fs::write(
        &desktop_engine,
        format!(
            "#!/bin/sh\ntouch '{}'\nprintf 'codex-cli 999.999.999\\n'\n",
            marker.display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&desktop_engine, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&applications, fs::Permissions::from_mode(0o775)).unwrap();
    }
    layout.codex_executable = directory.path().join("missing-path-cli");
    layout.codex_desktop_executable = Some(desktop_engine.clone());
    let scanner = FilesystemAgentScannerV1::new(layout, registry());

    let results = scanner.scan();
    assert!(results.iter().any(|result| matches!(
        &result.outcome,
        AgentDiscoveryOutcomeV1::Supported { installation }
            if installation.agent_id == "agent_codex_default"
                && installation.version == "not-probed"
    )));
    assert!(
        !marker.exists(),
        "discovery must not execute a version probe"
    );
    assert_eq!(
        scanner.codex_engine_target(hiroute_domain::AgentModelSurfaceV2::CodexDesktop),
        Some(desktop_engine)
    );
    assert_eq!(
        scanner.codex_engine_target(hiroute_domain::AgentModelSurfaceV2::CodexCli),
        Some(directory.path().join("missing-path-cli"))
    );
    assert_eq!(
        scanner.available_model_surfaces("agent_codex_default"),
        [hiroute_domain::AgentModelSurfaceV2::CodexDesktop]
            .into_iter()
            .collect()
    );
}

#[test]
fn filesystem_scanner_resolves_registered_provider_model_from_claude_alias_hint() {
    let directory = tempfile::tempdir().unwrap();
    let layout =
        layout_with_claude_version(directory.path(), CLAUDE_CODE_VERIFIED_VERSION_2_1_231_V1);
    write_secret_settings(
        &layout.claude_user_settings,
        json!({"env": {
            "ANTHROPIC_BASE_URL": "https://open.bigmodel.cn/api/anthropic",
            "ANTHROPIC_MODEL": "claude-opus-5",
            "ANTHROPIC_DEFAULT_OPUS_MODEL": "glm-5.3[1m]",
            "ANTHROPIC_AUTH_TOKEN": "secret-do-not-serialize"
        }}),
    );

    let results = FilesystemAgentScannerV1::new(layout, registry()).scan();
    let configuration = results
        .iter()
        .find(|result| outcome_agent_id(&result.outcome) == "agent_claude_default")
        .and_then(|result| result.claude_configuration.as_ref())
        .unwrap();

    assert_eq!(
        configuration.connection_option_id,
        "zhipu.coding-plan.cn.v1"
    );
    assert_eq!(
        configuration.model_configuration_id.as_deref(),
        Some("model.zhipu.glm-5.3")
    );
    assert_eq!(configuration.observed_model_id, "glm-5.3");
    assert_eq!(
        configuration.provider_model_alias_hints["configured_model"],
        "claude-opus-5"
    );
}

#[cfg(unix)]
#[test]
fn group_writable_claude_executable_is_probed_and_safe_configuration_is_discovered() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().unwrap();
    let layout = layout(directory.path());
    let version_probed = directory.path().join("claude-version-probed");
    let helper_executed = directory.path().join("claude-helper-executed");
    fs::write(
        &layout.claude_executable,
        format!(
            "#!/bin/sh\nprintf touched > '{}'\nprintf '2.1.231 (Claude Code)\\n'\n",
            version_probed.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&layout.claude_executable, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(
        layout.claude_executable.parent().unwrap(),
        fs::Permissions::from_mode(0o775),
    )
    .unwrap();
    write_secret_settings(
        &layout.claude_user_settings,
        json!({
            "apiKeyHelper": format!("touch {}", helper_executed.display()),
            "env": {
                "ANTHROPIC_BASE_URL": "https://open.bigmodel.cn/api/anthropic",
                "ANTHROPIC_MODEL": "glm-5.3",
                "ANTHROPIC_AUTH_TOKEN": "secret-never-serialized"
            }
        }),
    );

    let result = FilesystemAgentScannerV1::new(layout, registry())
        .scan()
        .into_iter()
        .find(|result| outcome_agent_id(&result.outcome) == "agent_claude_default")
        .unwrap();

    assert!(matches!(
        result.outcome,
        AgentDiscoveryOutcomeV1::Supported { ref installation }
            if installation.version == "2.1.231"
    ));
    assert!(result.claude_configuration.is_some());
    assert!(result.discovered_credential.is_some());
    assert!(result.configuration_issue.is_none());
    assert!(result.managed_launch.is_some());
    assert!(
        version_probed.exists(),
        "the bounded version probe did not run"
    );
    assert!(
        !helper_executed.exists(),
        "the configured helper was executed"
    );
    let encoded = serde_json::to_string(&result).unwrap();
    assert!(!encoded.contains("secret-never-serialized"));
    assert!(!encoded.contains(helper_executed.to_string_lossy().as_ref()));
}

#[cfg(unix)]
#[test]
fn non_executable_claude_is_reported_as_located_but_not_runnable() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().unwrap();
    let layout = layout(directory.path());
    fs::set_permissions(&layout.claude_executable, fs::Permissions::from_mode(0o600)).unwrap();
    let scanner = FilesystemAgentScannerV1::new(layout, registry());

    let result = scanner
        .scan()
        .into_iter()
        .find(|result| outcome_agent_id(&result.outcome) == "agent_claude_default")
        .unwrap();

    assert!(matches!(
        result.outcome,
        AgentDiscoveryOutcomeV1::ReportOnly {
            reason: AgentReportOnlyReasonV1::ExecutableNotRunnable,
            ..
        }
    ));
    assert!(
        scanner
            .available_model_surfaces("agent_claude_default")
            .is_empty()
    );
}

#[cfg(unix)]
#[test]
fn group_writable_claude_reports_unknown_configuration_without_exposing_or_importing_it() {
    use std::os::unix::fs::PermissionsExt;

    for (endpoint, model, issue) in [
        (
            "https://attacker.invalid",
            "glm-5.3",
            AgentReportOnlyReasonV1::UnregisteredEndpoint,
        ),
        (
            "https://open.bigmodel.cn/api/anthropic",
            "unknown-model",
            AgentReportOnlyReasonV1::UnregisteredModel,
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let layout = layout(directory.path());
        let version_probed = directory.path().join("claude-version-probed");
        let helper_executed = directory.path().join("claude-helper-executed");
        fs::write(
            &layout.claude_executable,
            format!(
                "#!/bin/sh\nprintf touched > '{}'\nprintf '2.1.231 (Claude Code)\\n'\n",
                version_probed.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&layout.claude_executable, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(
            layout.claude_executable.parent().unwrap(),
            fs::Permissions::from_mode(0o775),
        )
        .unwrap();
        write_secret_settings(
            &layout.claude_user_settings,
            json!({
                "apiKeyHelper": format!("touch {}", helper_executed.display()),
                "env": {
                    "ANTHROPIC_BASE_URL": endpoint,
                    "ANTHROPIC_MODEL": model,
                    "ANTHROPIC_AUTH_TOKEN": "unknown-secret-never-serialized"
                }
            }),
        );

        let result = FilesystemAgentScannerV1::new(layout, registry())
            .scan()
            .into_iter()
            .find(|result| outcome_agent_id(&result.outcome) == "agent_claude_default")
            .unwrap();
        assert!(matches!(
            result.outcome,
            AgentDiscoveryOutcomeV1::ReportOnly {
                ref version,
                ref reason,
                ..
            } if version == "2.1.231" && reason == &issue
        ));
        assert_eq!(result.configuration_issue.as_ref(), Some(&issue));
        assert!(result.claude_configuration.is_none());
        assert!(result.discovered_credential.is_none());
        assert!(result.managed_launch.is_some());
        assert!(
            version_probed.exists(),
            "the bounded version probe did not run"
        );
        assert!(
            !helper_executed.exists(),
            "the configured helper was executed"
        );
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(encoded.contains(match issue {
            AgentReportOnlyReasonV1::UnregisteredEndpoint => "unregistered_endpoint",
            AgentReportOnlyReasonV1::UnregisteredModel => "unregistered_model",
            _ => unreachable!(),
        }));
        assert!(!encoded.contains(endpoint));
        assert!(!encoded.contains(model));
        assert!(!encoded.contains("unknown-secret-never-serialized"));
        assert!(!encoded.contains(helper_executed.to_string_lossy().as_ref()));
    }
}

#[test]
fn claude_precedence_uses_process_then_launch_project_user_managed() {
    let directory = tempfile::tempdir().unwrap();
    let mut layout = layout(directory.path());
    let project = layout.claude_project_settings[0].clone();
    write_secret_settings(
        &project,
        json!({"env": {"ANTHROPIC_BASE_URL": "https://open.bigmodel.cn/api/anthropic", "ANTHROPIC_MODEL": "glm-5.3", "ANTHROPIC_AUTH_TOKEN": "project"}}),
    );
    layout = layout
        .with_process_value(
            "ANTHROPIC_BASE_URL",
            "https://open.bigmodel.cn/api/anthropic",
        )
        .with_process_value("ANTHROPIC_MODEL", "glm-5.3")
        .with_process_value("ANTHROPIC_AUTH_TOKEN", "process");
    let scanner = FilesystemAgentScannerV1::new(layout, registry());
    let results = scanner.scan();
    let claude = results
        .into_iter()
        .find(|result| outcome_agent_id(&result.outcome) == "agent_claude_default")
        .unwrap();
    let descriptor = claude.discovered_credential.unwrap();
    assert_eq!(
        descriptor.discovered_source_ref,
        "claude/process-environment"
    );
    assert_eq!(
        scanner
            .read_discovered_secret(&descriptor)
            .unwrap()
            .expose(),
        b"process"
    );
}

#[test]
fn codex_version_is_not_probed_and_unregistered_endpoint_stays_report_only() {
    let directory = tempfile::tempdir().unwrap();
    let layout = layout(directory.path());
    write_executable(&layout.codex_executable, "codex-cli 0.116.1");
    write_secret_settings(
        &layout.claude_user_settings,
        json!({"env": {"ANTHROPIC_BASE_URL": "https://attacker.invalid", "ANTHROPIC_MODEL": "qwen3-coder-plus", "ANTHROPIC_AUTH_TOKEN": "secret"}}),
    );
    let results = FilesystemAgentScannerV1::new(layout, registry()).scan();
    assert!(results.iter().any(|result| matches!(
        &result.outcome, AgentDiscoveryOutcomeV1::Supported { installation }
            if installation.version == "not-probed"
            && installation.require_action(hiroute_domain::AgentAction::RestoreModel).is_ok()
            && installation.require_action(hiroute_domain::AgentAction::ConfigureModel).is_err()
    )));
    assert!(results.iter().any(|result| matches!(
        result.outcome,
        AgentDiscoveryOutcomeV1::ReportOnly {
            reason: AgentReportOnlyReasonV1::UnregisteredEndpoint,
            ..
        }
    )));
}

#[test]
fn claude_endpoint_requires_the_exact_registered_sdk_messages_url() {
    for near_miss in [
        "https://coding.dashscope.aliyuncs.com",
        "https://coding.dashscope.aliyuncs.com/",
        "https://coding.dashscope.aliyuncs.com/apps/anthropic/",
        "https://coding.dashscope.aliyuncs.com/apps/anthropic/v1",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let layout = layout(directory.path());
        write_secret_settings(
            &layout.claude_user_settings,
            json!({"env": {
                "ANTHROPIC_BASE_URL": near_miss,
                "ANTHROPIC_MODEL": "qwen3-coder-plus",
                "ANTHROPIC_AUTH_TOKEN": "secret"
            }}),
        );
        let result = FilesystemAgentScannerV1::new(layout, registry())
            .scan()
            .into_iter()
            .find(|result| outcome_agent_id(&result.outcome) == "agent_claude_default")
            .unwrap();
        assert!(
            matches!(
                result.outcome,
                AgentDiscoveryOutcomeV1::ReportOnly {
                    reason: AgentReportOnlyReasonV1::UnregisteredEndpoint,
                    ..
                }
            ),
            "accepted endpoint near miss {near_miss}"
        );
        assert!(result.discovered_credential.is_none());
    }
}

#[test]
fn process_discovery_reference_is_presence_bound_and_secret_is_read_only_on_demand() {
    let directory = tempfile::tempdir().unwrap();
    let first_layout = layout(directory.path())
        .with_process_value(
            "ANTHROPIC_BASE_URL",
            "https://open.bigmodel.cn/api/anthropic",
        )
        .with_process_value("ANTHROPIC_MODEL", "glm-5.3")
        .with_process_value("ANTHROPIC_AUTH_TOKEN", "first-secret");
    let first = FilesystemAgentScannerV1::new(first_layout.clone(), registry());
    let descriptor = first
        .scan()
        .into_iter()
        .find(|result| outcome_agent_id(&result.outcome) == "agent_claude_default")
        .unwrap()
        .discovered_credential
        .unwrap();
    let changed = FilesystemAgentScannerV1::new(
        first_layout.with_process_value("ANTHROPIC_AUTH_TOKEN", "second-secret"),
        registry(),
    );
    assert_eq!(
        changed
            .read_discovered_secret(&descriptor)
            .unwrap()
            .expose(),
        b"second-secret"
    );
}

#[test]
fn managed_2_1_231_sanitizes_inherited_env_and_keeps_compute_discovery() {
    let directory = tempfile::tempdir().unwrap();
    let layout =
        layout_with_claude_version(directory.path(), CLAUDE_CODE_VERIFIED_VERSION_2_1_231_V1)
            .with_process_value(
                "ANTHROPIC_BASE_URL",
                "https://open.bigmodel.cn/api/anthropic",
            )
            .with_process_value("ANTHROPIC_MODEL", "glm-5.3")
            .with_process_value("ANTHROPIC_AUTH_TOKEN", "process-secret")
            .with_process_value("CLAUDE_CODE_USE_BEDROCK", "1");
    let scanner = FilesystemAgentScannerV1::new(layout, registry());
    let result = scanner
        .scan()
        .into_iter()
        .find(|result| outcome_agent_id(&result.outcome) == "agent_claude_default")
        .unwrap();

    assert!(matches!(
        result.outcome,
        AgentDiscoveryOutcomeV1::Supported { .. }
    ));
    assert!(result.claude_configuration.is_some());
    assert!(result.discovered_credential.is_some());
    let preflight = result.managed_launch.as_ref().unwrap();
    assert!(preflight.is_launchable());
    assert_eq!(preflight.warnings.len(), 1);
    assert_eq!(preflight.warnings[0].code, MANAGED_LAUNCH_ENV_SANITIZED_V1);
    assert_eq!(
        preflight.inherited_environment_removals,
        BTreeSet::from([
            "ANTHROPIC_AUTH_TOKEN".to_owned(),
            "ANTHROPIC_BASE_URL".to_owned(),
            "ANTHROPIC_MODEL".to_owned(),
            "CLAUDE_CODE_USE_BEDROCK".to_owned(),
        ])
    );
    let encoded = serde_json::to_string(&result).unwrap();
    assert!(!encoded.contains("process-secret"));
}

#[test]
fn managed_2_1_231_isolates_user_settings_and_keeps_compute_discovery() {
    let directory = tempfile::tempdir().unwrap();
    let layout =
        layout_with_claude_version(directory.path(), CLAUDE_CODE_VERIFIED_VERSION_2_1_231_V1);
    write_secret_settings(
        &layout.claude_user_settings,
        json!({
            "apiKeyHelper": "do-not-run --secret",
            "env": {
                "ANTHROPIC_BASE_URL": "https://open.bigmodel.cn/api/anthropic",
                "ANTHROPIC_MODEL": "claude-opus-5",
                "ANTHROPIC_DEFAULT_OPUS_MODEL": "glm-5.3[1m]",
                "ANTHROPIC_AUTH_TOKEN": "user-secret"
            }
        }),
    );
    let result = FilesystemAgentScannerV1::new(layout, registry())
        .scan()
        .into_iter()
        .find(|result| outcome_agent_id(&result.outcome) == "agent_claude_default")
        .unwrap();

    assert!(result.claude_configuration.is_some());
    assert!(result.discovered_credential.is_some());
    let preflight = result.managed_launch.as_ref().unwrap();
    assert!(preflight.is_launchable());
    assert!(preflight.conflicts.is_empty());
    let encoded = serde_json::to_string(&result).unwrap();
    assert!(!encoded.contains("user-secret"));
    assert!(!encoded.contains("do-not-run"));
}

#[test]
fn managed_2_1_231_blocks_auth_routing_fields_from_every_nonprocess_layer() {
    let directory = tempfile::tempdir().unwrap();
    let mut layout =
        layout_with_claude_version(directory.path(), CLAUDE_CODE_VERIFIED_VERSION_2_1_231_V1);
    let launch = directory.path().join("launch/settings.json");
    let managed = directory.path().join("managed/settings.json");
    layout.claude_launch_settings = Some(launch.clone());
    layout.claude_managed_settings = vec![managed.clone()];
    write_secret_settings(&launch, json!({"apiKeyHelper": "must-not-run"}));
    write_secret_settings(
        &layout.claude_project_settings[0],
        json!({"env": {"ANTHROPIC_BASE_URL": "https://attacker.invalid"}}),
    );
    write_secret_settings(
        &layout.claude_user_settings,
        json!({"env": {"ANTHROPIC_MODEL": "untrusted-model"}}),
    );
    write_secret_settings(&managed, json!({"env": {"CLAUDE_CODE_USE_VERTEX": "1"}}));

    let result = FilesystemAgentScannerV1::new(layout, registry())
        .scan()
        .into_iter()
        .find(|result| outcome_agent_id(&result.outcome) == "agent_claude_default")
        .unwrap();
    let preflight = result.managed_launch.unwrap();
    assert_eq!(
        preflight
            .conflicts
            .iter()
            .map(|conflict| conflict.layer)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([ConfigLayerV1::Launch, ConfigLayerV1::Managed])
    );
    assert!(
        preflight
            .conflicts
            .iter()
            .all(|conflict| conflict.code == AGENT_AUTH_PRECEDENCE_CONFLICT_V1)
    );
    assert!(
        !serde_json::to_string(&preflight)
            .unwrap()
            .contains("must-not-run")
    );
}

#[test]
fn managed_2_1_231_isolates_multiple_project_settings() {
    let directory = tempfile::tempdir().unwrap();
    let mut layout =
        layout_with_claude_version(directory.path(), CLAUDE_CODE_VERIFIED_VERSION_2_1_231_V1);
    let second = directory.path().join("project-2/.claude/settings.json");
    layout.claude_project_settings.push(second.clone());
    for (path, endpoint, token) in [
        (
            &layout.claude_project_settings[0],
            "https://open.bigmodel.cn/api/anthropic",
            "first-secret",
        ),
        (&second, "https://attacker.invalid", "second-secret"),
    ] {
        write_secret_settings(
            path,
            json!({"env": {
                "ANTHROPIC_BASE_URL": endpoint,
                "ANTHROPIC_MODEL": "glm-5.3",
                "ANTHROPIC_AUTH_TOKEN": token,
            }}),
        );
    }
    let result = FilesystemAgentScannerV1::new(layout, registry())
        .scan()
        .into_iter()
        .find(|result| outcome_agent_id(&result.outcome) == "agent_claude_default")
        .unwrap();

    assert!(matches!(
        result.outcome,
        AgentDiscoveryOutcomeV1::Supported { .. }
    ));
    let AgentDiscoveryOutcomeV1::Supported { installation } = &result.outcome else {
        unreachable!()
    };
    assert!(
        installation
            .require_action(hiroute_domain::AgentAction::RestoreModel)
            .is_err()
    );
    assert!(result.claude_configuration.is_none());
    assert!(result.discovered_credential.is_none());
    let preflight = result.managed_launch.unwrap();
    assert!(preflight.is_launchable());
    assert!(preflight.conflicts.is_empty());
    let encoded = serde_json::to_string(&preflight).unwrap();
    assert!(!encoded.contains("first-secret"));
    assert!(!encoded.contains("second-secret"));
}

#[test]
fn managed_launch_is_exactly_2_1_231_only() {
    for (version, supported, managed) in [
        (CLAUDE_CODE_VERIFIED_VERSION_V1, true, false),
        ("2.1.230", true, false),
        (CLAUDE_CODE_VERIFIED_VERSION_2_1_231_V1, true, true),
        ("2.1.232", true, false),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let result = FilesystemAgentScannerV1::new(
            layout_with_claude_version(directory.path(), version),
            registry(),
        )
        .scan()
        .into_iter()
        .find(|result| outcome_agent_id(&result.outcome) == "agent_claude_default")
        .unwrap();
        assert_eq!(
            matches!(result.outcome, AgentDiscoveryOutcomeV1::Supported { .. }),
            supported,
            "unexpected support state for {version}"
        );
        assert_eq!(result.managed_launch.is_some(), managed, "{version}");
    }
}

#[cfg(unix)]
#[test]
fn symlink_fails_closed_but_readable_0644_config_does_not_require_hardening() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let directory = tempfile::tempdir().unwrap();
    let layout = layout(directory.path());
    let target = directory.path().join("target.json");
    write_secret_settings(&target, json!({"env": {}}));
    fs::create_dir_all(layout.claude_user_settings.parent().unwrap()).unwrap();
    symlink(&target, &layout.claude_user_settings).unwrap();
    let result = FilesystemAgentScannerV1::new(layout.clone(), registry())
        .scan()
        .into_iter()
        .find(|result| outcome_agent_id(&result.outcome) == "agent_claude_default")
        .unwrap();
    assert!(matches!(
        result.outcome,
        AgentDiscoveryOutcomeV1::ReportOnly {
            reason: AgentReportOnlyReasonV1::SymlinkConfig,
            ..
        }
    ));

    fs::remove_file(&layout.claude_user_settings).unwrap();
    write_secret_settings(&layout.claude_user_settings, json!({"env": {}}));
    fs::set_permissions(
        &layout.claude_user_settings,
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    let result = FilesystemAgentScannerV1::new(layout.clone(), registry())
        .scan()
        .into_iter()
        .find(|result| outcome_agent_id(&result.outcome) == "agent_claude_default")
        .unwrap();
    assert!(matches!(
        result.outcome,
        AgentDiscoveryOutcomeV1::Supported { .. }
    ));
    assert!(result.permission_hardening.is_none());
    assert!(result.configuration_issue.is_none());
    assert_eq!(
        fs::metadata(&layout.claude_user_settings)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o644
    );
    // Explicit file hardening remains independently testable; scanning never invokes it.
    let finding = ClaudeSource::File(ConfigLayerV1::User, layout.claude_user_settings.clone())
        .permission_hardening_required()
        .unwrap()
        .unwrap();
    assert_eq!(finding.display_path, "~/.claude/settings.json");
    let scanner = FilesystemAgentScannerV1::new(layout, registry());
    assert_eq!(
        scanner.harden_discovered_permissions(&finding).unwrap(),
        PermissionHardeningOutcomeV1::Hardened
    );
    assert_eq!(
        fs::metadata(&scanner.layout.claude_user_settings)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        scanner.harden_discovered_permissions(&finding).unwrap(),
        PermissionHardeningOutcomeV1::AlreadyHardened
    );
}

#[cfg(unix)]
#[test]
fn hardlink_and_identity_drift_fail_before_permission_change_or_secret_read() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().unwrap();
    let layout = layout(directory.path());
    let target = directory.path().join("hardlink-target.json");
    write_secret_settings(
        &target,
        json!({"env": {"ANTHROPIC_AUTH_TOKEN": "never-read"}}),
    );
    fs::create_dir_all(layout.claude_user_settings.parent().unwrap()).unwrap();
    fs::hard_link(&target, &layout.claude_user_settings).unwrap();
    let hardlink = FilesystemAgentScannerV1::new(layout.clone(), registry())
        .scan()
        .into_iter()
        .find(|result| outcome_agent_id(&result.outcome) == "agent_claude_default")
        .unwrap();
    assert!(matches!(
        hardlink.outcome,
        AgentDiscoveryOutcomeV1::ReportOnly {
            reason: AgentReportOnlyReasonV1::ConfigUnavailable,
            ..
        }
    ));
    assert!(hardlink.discovered_credential.is_none());

    fs::remove_file(&layout.claude_user_settings).unwrap();
    fs::remove_file(&target).unwrap();
    write_secret_settings(&layout.claude_user_settings, json!({"env": {}}));
    fs::set_permissions(
        &layout.claude_user_settings,
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    let scanner = FilesystemAgentScannerV1::new(layout.clone(), registry());
    let finding = ClaudeSource::File(ConfigLayerV1::User, layout.claude_user_settings.clone())
        .permission_hardening_required()
        .unwrap()
        .unwrap();
    fs::remove_file(&layout.claude_user_settings).unwrap();
    write_secret_settings(&layout.claude_user_settings, json!({"env": {}}));
    fs::set_permissions(
        &layout.claude_user_settings,
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert!(matches!(
        scanner.harden_discovered_permissions(&finding),
        Err(AgentFilesystemScanError::SourceChanged)
    ));
    assert_eq!(
        fs::metadata(&layout.claude_user_settings)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o644
    );
}

#[test]
fn conflicting_same_layer_tokens_block_main_configuration_without_hiding_installation() {
    let directory = tempfile::tempdir().unwrap();
    let mut layout = layout(directory.path());
    let second = directory.path().join("project-2/.claude/settings.json");
    layout.claude_project_settings.push(second.clone());
    for (path, token) in [
        (&layout.claude_project_settings[0], "first-secret"),
        (&second, "second-secret"),
    ] {
        write_secret_settings(
            path,
            json!({"env": {
                "ANTHROPIC_BASE_URL": "https://coding.dashscope.aliyuncs.com/apps/anthropic",
                "ANTHROPIC_MODEL": "qwen3-coder-plus",
                "ANTHROPIC_AUTH_TOKEN": token,
            }}),
        );
    }
    let result = FilesystemAgentScannerV1::new(layout, registry())
        .scan()
        .into_iter()
        .find(|result| outcome_agent_id(&result.outcome) == "agent_claude_default")
        .unwrap();
    let AgentDiscoveryOutcomeV1::Supported { installation } = &result.outcome else {
        panic!("physical installation remains known");
    };
    assert!(
        installation
            .require_action(hiroute_domain::AgentAction::RestoreModel)
            .is_err()
    );
    assert!(
        installation
            .capability_evidence
            .iter()
            .any(|proof| proof.capability
                == hiroute_domain::AgentCapability::EffectiveConfiguration
                && proof.reason
                    == Some(hiroute_domain::CapabilityReason::HigherPrecedenceConflict))
    );
    assert!(result.claude_configuration.is_none());
    assert!(result.discovered_credential.is_none());
    let encoded = serde_json::to_string(&result).unwrap();
    assert!(!encoded.contains("first-secret"));
    assert!(!encoded.contains("second-secret"));
}

#[test]
fn claude_native_renderer_preserves_unowned_settings_and_replaces_auth_atomically() {
    let directory = tempfile::tempdir().unwrap();
    let layout =
        layout_with_claude_version(directory.path(), CLAUDE_CODE_VERIFIED_VERSION_2_1_231_V1);
    write_secret_settings(
        &layout.claude_user_settings,
        json!({
            "theme": "dark",
            "env": {
                "ANTHROPIC_BASE_URL": "https://open.bigmodel.cn/api/anthropic",
                "ANTHROPIC_MODEL": "glm-5.3",
                "ANTHROPIC_DEFAULT_OPUS_MODEL": "glm-5.3[1m]",
                "ANTHROPIC_AUTH_TOKEN": "secret-never-journaled",
                "UNRELATED": "keep-me"
            }
        }),
    );
    let scanner = FilesystemAgentScannerV1::new(layout.clone(), registry());
    let discovery = scanner
        .scan()
        .into_iter()
        .find(|value| outcome_agent_id(&value.outcome) == "agent_claude_default")
        .unwrap();
    let AgentDiscoveryOutcomeV1::Supported { installation } = discovery.outcome else {
        panic!("exact Claude installation must be supported");
    };
    let current = hiroute_domain::AgentConfigDocumentV1 {
        fields: installation
            .effective_config
            .iter()
            .map(|(path, value)| (path.clone(), value.value.clone()))
            .collect(),
    };
    let alias = "hiroute/0123456789abcdef";
    let desired = BTreeMap::from([
        (
            "env.ANTHROPIC_BASE_URL".to_owned(),
            Some(json!("http://127.0.0.1:35837")),
        ),
        ("env.ANTHROPIC_MODEL".to_owned(), Some(json!(alias))),
        (
            "env.ANTHROPIC_DEFAULT_OPUS_MODEL".to_owned(),
            Some(json!(alias)),
        ),
        (
            "env.ANTHROPIC_DEFAULT_SONNET_MODEL".to_owned(),
            Some(json!(alias)),
        ),
        (
            "env.ANTHROPIC_DEFAULT_HAIKU_MODEL".to_owned(),
            Some(json!(alias)),
        ),
        (
            "env.ANTHROPIC_SMALL_FAST_MODEL".to_owned(),
            Some(json!(alias)),
        ),
        (
            "apiKeyHelper".to_owned(),
            Some(json!({
                "executable": "/opt/hiroute/bin/hiroute",
                "argv": [
                    hiroute_domain::HIDDEN_AGENT_GRANT_HELPER_VERB_V1,
                    "agent-connection/agent_claude_default/claude-messages-v1"
                ]
            })),
        ),
        ("hiroute.auth_environment".to_owned(), None),
    ]);
    let writable = installation.writable_values(&desired).unwrap();
    let change = hiroute_domain::AgentConfigChangeV1::preview(&current, writable).unwrap();
    let rendered = scanner.render_claude_user_config_change(&change).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&rendered).unwrap();

    assert_eq!(value["theme"], "dark");
    assert_eq!(value["env"]["UNRELATED"], "keep-me");
    for name in [
        "ANTHROPIC_MODEL",
        "ANTHROPIC_DEFAULT_OPUS_MODEL",
        "ANTHROPIC_DEFAULT_SONNET_MODEL",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL",
        "ANTHROPIC_SMALL_FAST_MODEL",
    ] {
        assert_eq!(value["env"][name], alias);
    }
    assert!(value["env"].get("ANTHROPIC_AUTH_TOKEN").is_none());
    let helper = value["apiKeyHelper"].as_str().unwrap();
    assert!(helper.contains(hiroute_domain::HIDDEN_AGENT_GRANT_HELPER_VERB_V1));
    assert!(!String::from_utf8_lossy(&rendered).contains("secret-never-journaled"));

    fs::write(&layout.claude_user_settings, rendered.as_slice()).unwrap();
    assert!(
        scanner
            .claude_user_config_change_is_applied(&change)
            .unwrap()
    );
    let mut rewritten = value;
    rewritten["claude_unowned_runtime_field"] = json!(true);
    fs::write(
        &layout.claude_user_settings,
        serde_json::to_vec(&rewritten).unwrap(),
    )
    .unwrap();
    assert!(
        scanner
            .claude_user_config_change_is_applied(&change)
            .unwrap()
    );
    rewritten["env"]["ANTHROPIC_MODEL"] = json!("not-the-managed-alias");
    fs::write(
        &layout.claude_user_settings,
        serde_json::to_vec_pretty(&rewritten).unwrap(),
    )
    .unwrap();
    assert!(
        !scanner
            .claude_user_config_change_is_applied(&change)
            .unwrap()
    );
}

#[test]
fn filesystem_scan_keeps_missing_installation_and_other_agent_result() {
    let directory = tempfile::tempdir().unwrap();
    let mut layout = layout(directory.path());
    layout.codex_executable = directory.path().join("not-installed");
    let results = FilesystemAgentScannerV1::new(layout, registry()).scan();
    assert_eq!(results.len(), 2);
    assert!(results.iter().any(|result| matches!(
        &result.outcome,
        AgentDiscoveryOutcomeV1::ReportOnly {
            kind: AgentKindV1::Codex,
            reason: AgentReportOnlyReasonV1::NotFoundInScope,
            ..
        }
    )));
    assert!(
        results.iter().any(|result| matches!(&result.outcome,
            AgentDiscoveryOutcomeV1::Supported { installation }
                if installation.profile.kind == AgentKindV1::ClaudeCode)),
        "independent Claude discovery unexpectedly degraded: {results:#?}"
    );
}

#[test]
fn filesystem_claude_managed_source_wins_for_both_values_and_protected_reread() {
    let directory = tempfile::tempdir().unwrap();
    let mut layout =
        layout(directory.path()).with_process_value("ANTHROPIC_AUTH_TOKEN", "process-secret");
    let managed = directory.path().join("managed-settings.json");
    write_secret_settings(
        &managed,
        json!({"env": {
            "ANTHROPIC_BASE_URL": "https://open.bigmodel.cn/api/anthropic",
            "ANTHROPIC_MODEL": "glm-5.3", "ANTHROPIC_AUTH_TOKEN": "managed-secret"
        }}),
    );
    layout.claude_managed_settings.push(managed);
    let scanner = FilesystemAgentScannerV1::new(layout, registry());
    let results = scanner.scan();
    let claude = results
        .iter()
        .find(|result| outcome_agent_id(&result.outcome) == "agent_claude_default")
        .unwrap();
    let AgentDiscoveryOutcomeV1::Supported { installation } = &claude.outcome else {
        panic!("valid managed settings");
    };
    assert_eq!(
        installation.effective_config["env.ANTHROPIC_BASE_URL"].layer,
        ConfigLayerV1::Managed
    );
    let credential = claude.discovered_credential.as_ref().unwrap();
    assert_eq!(
        scanner.read_discovered_secret(credential).unwrap().expose(),
        b"managed-secret"
    );
    let public = serde_json::to_string(&results).unwrap();
    assert!(!public.contains("managed-secret"));
    assert!(!public.contains("process-secret"));
}

#[test]
fn codex_subscription_identity_is_stable_across_token_rotation() {
    let directory = tempfile::tempdir().unwrap();
    let layout = layout(directory.path());
    let auth_path = layout.codex_user_config.parent().unwrap().join("auth.json");
    write_secret_settings(
        &auth_path,
        json!({"auth_mode": "chatgpt", "last_refresh": "one", "tokens": {
            "access_token": "first-private-token", "account_id": "personal-account"
        }}),
    );
    let scanner = FilesystemAgentScannerV1::new(layout, registry());
    let first = scanner.codex_subscription_source().unwrap().unwrap();

    write_secret_settings(
        &auth_path,
        json!({"auth_mode": "chatgpt", "last_refresh": "two", "tokens": {
            "access_token": "different-private-token-with-another-length",
            "account_id": "personal-account"
        }}),
    );
    let rotated = scanner.codex_subscription_source().unwrap().unwrap();

    assert_eq!(rotated.descriptor(), first.descriptor());
    assert_eq!(rotated.evidence_digest(), first.evidence_digest());
    assert_eq!(rotated.source_path(), first.source_path());
}

#[path = "source_candidates/tests.rs"]
mod source_candidate_tests;

#[path = "filesystem_layer_tests.rs"]
mod layer_tests;
