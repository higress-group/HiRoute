//! Skill-only Preview must consume an actual explicit check on Windows too.
use super::*;
use hiroute_domain::AgentAction;
use std::io::Write;

#[test]
fn windows_skill_only_settings_require_fresh_collaboration_not_authentication() {
    let root = super::super::probe_files::temporary("hiroute-skill-settings-").unwrap();
    let config = root.path().join("config.toml");
    // A collaboration-only edit must not parse or overwrite unrelated model configuration.
    super::super::probe_files::file(&config)
        .unwrap()
        .write_all(b"unrelated = [invalid TOML\n")
        .unwrap();
    let mut layout = super::super::AgentFilesystemLayoutV1::from_process(root.path(), root.path());
    layout.codex_executable = std::env::current_exe().unwrap();
    layout.codex_user_config = config.clone();
    let registry: hiroute_domain::ConnectorRegistryBundleV1 = serde_json::from_slice(
        include_bytes!("../../../../assets/connector-registry/current/registry-seed.json"),
    )
    .unwrap();
    let models: hiroute_domain::ReleaseModelDataBundleV2 = serde_json::from_slice(include_bytes!(
        "../../../../assets/release-facts/current/bundle/model-data.json"
    ))
    .unwrap();
    let registry =
        super::super::ClaudeRegistrationIndexV1::from_verified_model_data(&registry, &models.data)
            .unwrap();
    for (collaboration, age, allowed) in [(true, 0, true), (false, 0, false), (true, 301, false)] {
        let scanner = super::super::FilesystemAgentScannerV1::new(layout.clone(), registry.clone())
            .with_codex_ingress_evidence(CodexIngressEvidence {
                observed: Instant::now() - Duration::from_secs(age),
                observed_at: now(),
                collaboration,
            });
        let installation = scanner
            .skill_settings_installation("agent_codex_default")
            .unwrap();
        assert_eq!(
            installation
                .require_action(AgentAction::InstallCollaborationSkill)
                .is_ok(),
            allowed
        );
        assert_eq!(
            std::fs::read(&config).unwrap(),
            b"unrelated = [invalid TOML\n"
        );
    }
}
