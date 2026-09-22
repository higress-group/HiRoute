use super::*;

#[test]
fn managed_launch_sanitizes_inherited_env_and_keeps_compute_discovery() {
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
fn managed_launch_isolates_user_settings_and_keeps_compute_discovery() {
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
fn managed_launch_blocks_auth_routing_fields_from_every_nonprocess_layer() {
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
fn managed_launch_isolates_multiple_project_settings() {
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
fn managed_launch_accepts_old_new_and_unparseable_versions() {
    for (version, supported, managed) in [
        (CLAUDE_CODE_VERIFIED_VERSION_V1, true, true),
        ("0.0.1", true, true),
        (CLAUDE_CODE_VERIFIED_VERSION_2_1_231_V1, true, true),
        ("99.1.2", true, true),
        ("", true, true),
        ("unparseable version", true, true),
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
