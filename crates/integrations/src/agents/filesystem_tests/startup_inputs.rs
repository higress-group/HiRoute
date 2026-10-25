use super::*;

#[test]
fn configuration_priming_preserves_exact_inputs_and_permission_findings() {
    let directory = tempfile::tempdir().unwrap();
    let layout = layout(directory.path());
    write_secret_settings(
        &layout.claude_user_settings,
        json!({"env": {
            "ANTHROPIC_BASE_URL": "https://open.bigmodel.cn/api/anthropic",
            "ANTHROPIC_MODEL": "claude-opus-5",
            "ANTHROPIC_DEFAULT_OPUS_MODEL": "glm-5.3[1m]",
            "ANTHROPIC_AUTH_TOKEN": "test-only-priming-input"
        }}),
    );
    let settings = layout.claude_user_settings.clone();
    let scanner = FilesystemAgentScannerV1::new(layout, registry());
    let (credential, finding) = scanner.claude_protected_configuration_inputs();
    let complete = scanner
        .scan()
        .into_iter()
        .find(|value| outcome_agent_id(&value.outcome) == "agent_claude_default")
        .unwrap();
    assert!(credential.is_some());
    assert_eq!(credential, complete.discovered_credential);
    assert_eq!(finding, complete.permission_hardening);
    assert_eq!(
        scanner
            .read_discovered_secret(credential.as_ref().unwrap())
            .unwrap()
            .expose(),
        b"test-only-priming-input"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&settings, fs::Permissions::from_mode(0o644)).unwrap();
        let (credential, finding) = scanner.claude_protected_configuration_inputs();
        let complete = scanner
            .scan()
            .into_iter()
            .find(|value| outcome_agent_id(&value.outcome) == "agent_claude_default")
            .unwrap();
        // Priming follows the existing passive native-config policy; it must
        // neither invent a new permission gate nor silently harden the file.
        assert_eq!(credential, complete.discovered_credential);
        assert_eq!(finding, complete.permission_hardening);
        assert_eq!(
            fs::metadata(&settings).unwrap().permissions().mode() & 0o777,
            0o644
        );
    }
}
