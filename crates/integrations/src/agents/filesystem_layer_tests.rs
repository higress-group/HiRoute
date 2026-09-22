use super::*;

#[test]
fn main_claude_project_layer_and_managed_launch_are_version_independent() {
    for version in [CLAUDE_CODE_VERIFIED_VERSION_2_1_231_V1, "9.9.999"] {
        let directory = tempfile::tempdir().unwrap();
        let layout = layout_with_claude_version(directory.path(), version);
        write_secret_settings(
            &layout.claude_user_settings,
            json!({"env":{"ANTHROPIC_MODEL":"user-model"}}),
        );
        write_secret_settings(
            &layout.claude_project_settings[0],
            json!({"env":{"ANTHROPIC_MODEL":"project-model"}}),
        );
        let found = FilesystemAgentScannerV1::new(layout, registry())
            .scan()
            .into_iter()
            .find(|result| outcome_agent_id(&result.outcome) == "agent_claude_default")
            .unwrap();
        let AgentDiscoveryOutcomeV1::Supported { installation } = &found.outcome else {
            panic!("main-agent layer observation remains available");
        };
        let field = &installation.effective_config["env.ANTHROPIC_MODEL"];
        assert_eq!(field.layer, ConfigLayerV1::Project);
        assert_eq!(field.value, json!("project-model"));
        assert!(
            installation
                .require_action(hiroute_domain::AgentAction::RestoreModel)
                .is_ok()
        );
        assert!(found.managed_launch.is_some());
    }
}

#[test]
fn claude_local_settings_override_each_field_and_source_reread_binds_shadowed_layers() {
    for reverse in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let mut layout =
            layout_with_claude_version(directory.path(), CLAUDE_CODE_VERIFIED_VERSION_2_1_231_V1);
        let project = layout.claude_project_settings[0].clone();
        let local = project.parent().unwrap().join("settings.local.json");
        layout.claude_project_settings.push(local.clone());
        if reverse {
            layout.claude_project_settings.reverse();
        }
        write_secret_settings(
            &project,
            json!({"env":{
                "ANTHROPIC_BASE_URL":"https://open.bigmodel.cn/api/anthropic",
                "ANTHROPIC_MODEL":"project-model", "ANTHROPIC_AUTH_TOKEN":"project-secret"
            }}),
        );
        write_secret_settings(
            &local,
            json!({"env":{
                "ANTHROPIC_MODEL":"glm-5.3", "ANTHROPIC_AUTH_TOKEN":"local-secret"
            }}),
        );
        let scanner = FilesystemAgentScannerV1::new(layout, registry());
        let found = scanner
            .scan()
            .into_iter()
            .find(|result| outcome_agent_id(&result.outcome) == "agent_claude_default")
            .unwrap();
        let AgentDiscoveryOutcomeV1::Supported { installation } = &found.outcome else {
            panic!("known local override is not a conflict");
        };
        assert_eq!(
            installation.effective_config["env.ANTHROPIC_MODEL"].value,
            json!("glm-5.3")
        );
        assert!(
            installation
                .require_action(hiroute_domain::AgentAction::RestoreModel)
                .is_ok()
        );
        let candidates = scanner.claude_source_candidates().unwrap();
        assert_eq!(candidates.len(), 1);
        let selected = &candidates[0];
        let value = scanner.read_selected_claude_source(selected).unwrap();
        assert_eq!(
            value.endpoint.as_str(),
            "https://open.bigmodel.cn/api/anthropic"
        );
        assert_eq!(
            value.model.as_deref().map(|value| value.as_str()),
            Some("glm-5.3")
        );
        assert_eq!(value.credential.unwrap().expose(), b"local-secret");
        let descriptor = found.discovered_credential.unwrap();
        assert_eq!(
            scanner
                .read_discovered_secret(&descriptor)
                .unwrap()
                .expose(),
            b"local-secret"
        );
        // Even a changed shadowed value expires the selected snapshot; it may become active later.
        write_secret_settings(
            &project,
            json!({"env":{
                "ANTHROPIC_BASE_URL":"https://open.bigmodel.cn/api/anthropic",
                "ANTHROPIC_MODEL":"changed-shadowed-model", "ANTHROPIC_AUTH_TOKEN":"changed-shadowed-secret"
            }}),
        );
        assert!(scanner.read_selected_claude_source(selected).is_err());
        let after = scanner
            .scan()
            .into_iter()
            .find(|result| outcome_agent_id(&result.outcome) == "agent_claude_default")
            .unwrap();
        let AgentDiscoveryOutcomeV1::Supported {
            installation: after,
        } = after.outcome
        else {
            panic!("still installed");
        };
        assert_ne!(installation.observation_digest, after.observation_digest);
    }
}
