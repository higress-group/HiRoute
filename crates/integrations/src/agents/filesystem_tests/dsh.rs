//! DSH's supported passive source and CLI capability boundaries.
use super::*;

fn scanner(root: &Path) -> FilesystemAgentScannerV1 {
    let mut layout = layout(root);
    layout.dsh_executable = root.join("bin/dsh");
    fs::create_dir_all(&layout.dsh_config_root).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&layout.dsh_config_root, fs::Permissions::from_mode(0o700)).unwrap();
    }
    FilesystemAgentScannerV1::new(layout, registry())
}

fn write_patch(path: &Path, patch: serde_json::Value) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_vec(&patch).unwrap()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for directory in [
            path.parent().unwrap(),
            path.parent().unwrap().parent().unwrap(),
        ] {
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
        }
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }
}

#[test]
fn dsh_static_flow_sources_require_explicit_models_and_preservable_model_semantics() {
    let root = tempfile::tempdir().unwrap();
    let scanner = scanner(root.path());
    write_secret_settings(
        &scanner.layout.dsh_config_root.join(".credentials.yaml"),
        json!({"version":1,"refs":{"DSH_STATIC_SOURCE_TEST_KEY":"portable-token"}}),
    );
    let mut provider = json!({"api":"openai-responses","baseURL":"https://source.invalid/v1",
        "apiKeyEnv":"DSH_STATIC_SOURCE_TEST_KEY"});
    for models in [None, Some(json!([]))] {
        if let Some(models) = models {
            provider["models"] = models;
        }
        write_patch(
            &scanner.dsh_user_models_target(),
            json!([{"id":"llm-pi-ai","config":{"providers":{"native":provider}}}]),
        );
        assert!(
            scanner.dsh_api_sources().unwrap().is_empty(),
            "catalog-only providers are not explicit candidates"
        );
    }
    provider["models"] = json!([
        {"id":"portable","input":["text"]},
        {"id":"needs-compat","compat":{"supportsMaxOutputTokens":false}}
    ]);
    write_patch(
        &scanner.dsh_user_models_target(),
        json!([{"id":"llm-pi-ai","config":{"providers":{"native":provider}}}]),
    );
    let sources = scanner.dsh_api_sources().unwrap();
    let portable = sources.iter().find(|s| s.model_id == "portable").unwrap();
    assert!(portable.supported_auth && portable.credential.is_some());
    scanner.read_dsh_api_source(portable).unwrap();
    let special = sources
        .iter()
        .find(|s| s.model_id == "needs-compat")
        .unwrap();
    assert!(!special.supported_auth && special.credential.is_none());
    assert!(scanner.read_dsh_api_source(special).is_err());
}

#[test]
fn dsh_model_input_is_unknown_until_explicit_and_nonempty() {
    let root = tempfile::tempdir().unwrap();
    let scanner = scanner(root.path());
    write_patch(
        &scanner.dsh_user_models_target(),
        json!([{"id":"llm-pi-ai","config":{
            "providers":{"native":{"api":"openai-responses","baseURL":"https://source.invalid/v1",
                "defaultInput":["text"],"models":[
                    {"id":"inherited"},{"id":"empty","input":[]},
                    {"id":"text","input":["text"]},{"id":"image","input":["text","image"]}
                ]}}
        }}]),
    );
    let sources = scanner.dsh_api_sources().unwrap();
    for (id, expected) in [
        ("inherited", None),
        ("empty", None),
        ("text", Some(false)),
        ("image", Some(true)),
    ] {
        assert_eq!(
            sources.iter().find(|s| s.model_id == id).unwrap().vision,
            expected,
            "{id}"
        );
    }
}

#[cfg(unix)]
#[test]
fn dsh_collaboration_requires_enabled_standard_resources_and_ignores_model_only_changes() {
    use hiroute_domain::{AgentCapability, CapabilityState};
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let scanner = scanner(root.path());
    let cli = &scanner.layout.dsh_executable;
    fs::write(cli, "#!/bin/sh\nif [ \"$1\" = --version ]; then echo 'DSH fixture'; else printf '%s\\n' '- id: skill' '- id: skill-filesystem' '- id: tool-skill' '- id: tool-fs' '- id: tool-bash'; fi\n").unwrap();
    fs::set_permissions(cli, fs::Permissions::from_mode(0o700)).unwrap();
    scanner.check_dsh_collaboration(cli).unwrap();
    write_patch(
        &scanner.dsh_user_models_target(),
        json!([{"id":"llm-pi-ai","config":{"providers":{}}}]),
    );
    let AgentDiscoveryOutcomeV1::Supported { installation } =
        scanner.dsh_settings_discovery().outcome
    else {
        panic!("installed fixture")
    };
    assert!(installation.capability_evidence.iter().any(|p| p.capability
        == AgentCapability::SkillLoading
        && p.state == CapabilityState::Proven));
    for relative in [
        "cordis.patch.yml",
        "profiles/web/cordis.patch.yml",
        "profiles/acp/cordis.patch.yml",
    ] {
        let path = scanner.layout.dsh_config_root.join(relative);
        for module in [
            "skill-filesystem",
            "tool-skill",
            "tool-fs",
            "tool-bash",
            "skill",
        ] {
            write_patch(&path, json!([{"id":module,"disabled":true}]));
            assert!(
                scanner.check_dsh_collaboration(cli).is_err(),
                "{relative}: {module}"
            );
        }
        write_patch(
            &path,
            json!([{"id":"skill-filesystem","config":{"includeDefaultRoots":false}}]),
        );
        assert!(
            scanner.check_dsh_collaboration(cli).is_err(),
            "{relative}: standard user Skill root"
        );
        write_patch(
            &path,
            json!([{"id":"skill-filesystem","config":{"customSkillDirs":["/custom/skills"],"includeDefaultRoots":true}}]),
        );
        scanner.check_dsh_collaboration(cli).unwrap();
        fs::remove_file(path).unwrap();
    }
}

#[test]
fn dsh_static_import_is_secret_free_and_rejects_changed_credential_or_custom_auth() {
    let root = tempfile::tempdir().unwrap();
    let scanner = scanner(root.path());
    let target = scanner.dsh_user_models_target();
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    let patch = b"- id: llm-pi-ai\n  config:\n    providers:\n      native-source:\n        api: openai-responses\n        baseURL: http://127.0.0.1:4222/v1\n        apiKeyEnv: DSH_FIXTURE_KEY\n        models: [{id: native-model, contextWindow: 16384, maxTokens: 4096, input: [text]}]\n";
    fs::write(&target, patch).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for path in [
            target.parent().unwrap(),
            target.parent().unwrap().parent().unwrap(),
        ] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
    }
    let credentials = scanner.layout.dsh_config_root.join(".credentials.yaml");
    write_secret_settings(
        &credentials,
        json!({"version":1,"refs":{"DSH_FIXTURE_KEY":"saved-token"}}),
    );
    let selected = scanner.dsh_api_sources().unwrap().remove(0);
    let public = serde_json::to_string(&selected).unwrap();
    assert!(!public.contains("saved-token") && !public.contains("127.0.0.1"));
    let (endpoint, secret) = scanner.read_dsh_api_source(&selected).unwrap();
    assert_eq!(endpoint.as_str(), "http://127.0.0.1:4222/v1");
    assert_eq!(secret.expose(), b"saved-token");
    write_secret_settings(
        &credentials,
        json!({"version":1,"refs":{"DSH_FIXTURE_KEY":"rotated"}}),
    );
    assert!(matches!(
        scanner.read_dsh_api_source(&selected),
        Err(AgentFilesystemScanError::SourceChanged)
    ));
    fs::write(
        &target,
        String::from_utf8(patch.to_vec()).unwrap().replace(
            "        models:",
            "        headers: {Authorization: custom}\n        models:",
        ),
    )
    .unwrap();
    let selected = scanner.dsh_api_sources().unwrap().remove(0);
    assert!(!selected.supported_auth && selected.credential.is_none());
    assert!(scanner.read_dsh_api_source(&selected).is_err());
}

fn credentials_source(key: &str) -> serde_json::Value {
    json!({"id":"llm-pi-ai","config":{"providers":{"native":{
        "api":"openai-responses","baseURL":"https://credentials-source.invalid/v1",
        "apiKeyEnv":key,"models":[{"id":"explicit-model"}]
    }}}})
}

#[test]
fn dsh_credentials_use_the_effective_static_path_and_home_overrides() {
    let root = tempfile::tempdir().unwrap();
    let scanner = scanner(root.path());
    let key = "HIROUTE_DSH_CREDENTIAL_PATH_TEST_KEY";
    assert!(std::env::var_os(key).is_none());
    let file = root.path().join("custom.yaml");
    let home = root.path().join("custom-home");
    for (path, token) in [
        (
            scanner.layout.dsh_config_root.join(".credentials.yaml"),
            "default",
        ),
        (file.clone(), "explicit-path"),
        (home.join(".credentials.yaml"), "explicit-home"),
    ] {
        write_secret_settings(&path, json!({"version":1,"refs":{key:token}}));
    }
    for (profile_config, home_config, expected) in [
        (json!({"path":file}), None, "explicit-path"),
        (json!({"dshHome":home}), None, "explicit-home"),
        (
            json!({"path":file}),
            Some(json!({"dshHome":home})),
            "explicit-home",
        ),
        (
            json!({"dshHome":home}),
            Some(json!({"path":file,"dshHome":home})),
            "explicit-path",
        ),
        (json!({"path":file}), Some(json!({})), "default"),
    ] {
        write_patch(
            &scanner.dsh_user_models_target(),
            json!([
                credentials_source(key),
                {"id":"credentials","name":"@deepseek-ai/dsh-credentials-local","config":profile_config}
            ]),
        );
        write_patch(
            &scanner.layout.dsh_config_root.join("cordis.patch.yml"),
            home_config.map_or_else(
                || json!([]),
                |config| json!([{"id":"credentials","config":config}]),
            ),
        );
        let selected = scanner.dsh_api_sources().unwrap().remove(0);
        let public = serde_json::to_string(&selected).unwrap();
        assert!(!public.contains(file.to_str().unwrap()) && !public.contains("explicit-path"));
        assert_eq!(
            scanner.read_dsh_api_source(&selected).unwrap().1.expose(),
            expected.as_bytes()
        );
    }
}

#[test]
fn dsh_active_credentials_rotation_rejects_the_previous_import_selection() {
    let root = tempfile::tempdir().unwrap();
    let scanner = scanner(root.path());
    let key = "HIROUTE_DSH_CREDENTIAL_ROTATION_TEST_KEY";
    assert!(std::env::var_os(key).is_none());
    let default = scanner.layout.dsh_config_root.join(".credentials.yaml");
    let active = root.path().join("active.yaml");
    for path in [&default, &active] {
        write_secret_settings(path, json!({"version":1,"refs":{key:"original"}}));
    }
    write_patch(
        &scanner.dsh_user_models_target(),
        json!([credentials_source(key)]),
    );
    write_patch(
        &scanner.layout.dsh_config_root.join("cordis.patch.yml"),
        json!([{"id":"credentials","config":{"path":active}}]),
    );
    let selected = scanner.dsh_api_sources().unwrap().remove(0);
    write_secret_settings(&active, json!({"version":1,"refs":{key:"rotated"}}));
    assert!(matches!(
        scanner.read_dsh_api_source(&selected),
        Err(AgentFilesystemScanError::SourceChanged)
    ));
    let refreshed = scanner.dsh_api_sources().unwrap().remove(0);
    assert_eq!(
        scanner.read_dsh_api_source(&refreshed).unwrap().1.expose(),
        b"rotated"
    );
    write_secret_settings(&default, json!({"version":1,"refs":{key:"unused-default"}}));
    assert!(
        scanner.dsh_api_sources().unwrap().remove(0) == refreshed,
        "inactive default credentials must not change an effective-source selection"
    );
}

#[test]
fn dsh_disabled_removed_or_unknown_credentials_modules_block_import() {
    let root = tempfile::tempdir().unwrap();
    let scanner = scanner(root.path());
    let key = "HIROUTE_DSH_CREDENTIAL_REJECTION_TEST_KEY";
    assert!(std::env::var_os(key).is_none());
    write_secret_settings(
        &scanner.layout.dsh_config_root.join(".credentials.yaml"),
        json!({"version":1,"refs":{key:"must-not-import"}}),
    );
    for module in [
        json!({"id":"credentials","disabled":true}),
        json!({"id":"credentials","remove":true}),
        json!({"id":"credentials","name":"custom-credential-helper"}),
        json!({"id":"credentials","config":{"path":"relative.yaml"}}),
        json!({"id":"credentials","config":{"dshHome":"~/unresolved-home"}}),
        json!({"id":"credentials","config":{"path":false}}),
        json!({"id":"credentials","config":{"unknownProvider":"custom"}}),
        json!({"insert":[{"id":"credentials","name":"custom-credential-helper"}]}),
        json!({"id":"other","insert":[{"id":"credentials","name":"custom-credential-helper"}]}),
    ] {
        for home_layer in [false, true] {
            let profile = if home_layer {
                json!([credentials_source(key)])
            } else {
                json!([credentials_source(key), module])
            };
            write_patch(&scanner.dsh_user_models_target(), profile);
            write_patch(
                &scanner.layout.dsh_config_root.join("cordis.patch.yml"),
                if home_layer {
                    json!([module])
                } else {
                    json!([])
                },
            );
            assert!(
                scanner.dsh_api_sources().is_err(),
                "unsupported credentials module was importable"
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn dsh_effective_credentials_file_can_be_readable_without_chmod() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let scanner = scanner(root.path());
    let key = "HIROUTE_DSH_CREDENTIAL_PERMISSION_TEST_KEY";
    let active = root.path().join("active.yaml");
    for path in [
        &scanner.layout.dsh_config_root.join(".credentials.yaml"),
        &active,
    ] {
        write_secret_settings(path, json!({"version":1,"refs":{key:"private"}}));
    }
    write_patch(
        &scanner.dsh_user_models_target(),
        json!([
            credentials_source(key), {"id":"credentials","config":{"path":active}}
        ]),
    );
    fs::set_permissions(&active, fs::Permissions::from_mode(0o644)).unwrap();

    let selected = scanner.dsh_api_sources().unwrap().remove(0);
    scanner.read_dsh_api_source(&selected).unwrap();
    assert_eq!(
        fs::metadata(&active).unwrap().permissions().mode() & 0o777,
        0o644
    );
}

#[test]
fn dsh_inherited_environment_precedes_the_selected_credentials_file() {
    let key = "HIROUTE_DSH_CREDENTIAL_ENVIRONMENT_TEST_KEY";
    if std::env::var_os(key).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "agents::filesystem::tests::dsh::dsh_inherited_environment_precedes_the_selected_credentials_file"])
            .env(key, "inherited-key").output().unwrap();
        assert!(output.status.success());
        assert!(
            String::from_utf8(output.stdout)
                .unwrap()
                .contains("test result: ok. 1 passed;")
        );
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let scanner = scanner(root.path());
    let active = root.path().join("active.yaml");
    write_secret_settings(&active, json!({"version":1,"refs":{key:"stored-key"}}));
    write_patch(
        &scanner.dsh_user_models_target(),
        json!([
            credentials_source(key), {"id":"credentials","config":{"path":active}}
        ]),
    );
    let selected = scanner.dsh_api_sources().unwrap().remove(0);
    assert_eq!(
        scanner.read_dsh_api_source(&selected).unwrap().1.expose(),
        b"inherited-key"
    );
}

#[cfg(unix)]
#[test]
fn dsh_admission_uses_required_public_modules_not_cli_version() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let scanner = scanner(root.path());
    let cli = &scanner.layout.dsh_executable;
    // Version is diagnostic; the public dump demonstrates capabilities without inference.
    fs::write(cli,"#!/bin/sh\nif [ \"$1\" = --version ]; then echo 'DSH diagnostic-build'; else printf '%s\\n' '- id: llm-pi-ai' '- id: skill' '- id: skill-filesystem' '- id: tool-skill' '- id: tool-fs' '- id: tool-bash'; fi\n").unwrap();
    fs::set_permissions(cli, fs::Permissions::from_mode(0o700)).unwrap();
    scanner.check_dsh_model_configuration().unwrap();
    scanner.check_dsh_collaboration(cli).unwrap();
    fs::write(
        cli,
        "#!/bin/sh\nprintf '%s\\n' '- id: skill' '- id: skill-filesystem' '- id: tool-skill' '- id: tool-fs' '- id: tool-bash'\n",
    )
    .unwrap();
    assert!(scanner.check_dsh_model_configuration().is_err());
    // Model-provider capability is unnecessary for installing the collaboration Skill.
    scanner.check_dsh_collaboration(cli).unwrap();
    fs::write(cli, "#!/bin/sh\nprintf '%s\\n' '- id: llm-pi-ai'\n").unwrap();
    scanner.check_dsh_model_configuration().unwrap();
    assert!(scanner.check_dsh_collaboration(cli).is_err());
    let home = scanner.layout.dsh_config_root.join("cordis.patch.yml");
    fs::write(&home, b"- id: llm-pi-ai\n  config: {providers: {}}\n").unwrap();
    fs::set_permissions(&home, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(
        scanner.check_dsh_model_configuration().is_err(),
        "home layer must not shadow a managed Web provider"
    );
}

#[test]
fn dsh_home_default_guards_web_removal_without_requiring_cli() {
    let root = tempfile::tempdir().unwrap();
    let scanner = scanner(root.path());
    let home = scanner.layout.dsh_config_root.join("cordis.patch.yml");
    let namespace = "hiroute-main-test";
    let models = vec![hiroute_domain::AdditionalAgentModelV1 {
        alias: "route-a".into(),
        protocol: hiroute_domain::AgentIngressProtocolV1::Responses,
        context_window_tokens: 16384,
        max_output_tokens: 4096,
    }];
    let selected = hiroute_domain::additional_model_provider_for(namespace, &models, &models[0]);
    fs::write(
        &home,
        format!("- id: agent-default-model\n  config: {{provider: {selected}, model: route-a}}\n"),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&home, fs::Permissions::from_mode(0o600)).unwrap();
    }
    assert!(scanner.dsh_web_default_in_use(namespace, &[]).unwrap());
    assert!(!scanner.dsh_web_default_in_use(namespace, &models).unwrap());
    assert!(
        !scanner
            .dsh_web_default_in_use("another-owner", &[])
            .unwrap()
    );
    fs::write(
        &home,
        b"- id: agent-default-model\n  config: {provider: native, model: unchanged}\n",
    )
    .unwrap();
    assert!(!scanner.dsh_web_default_in_use(namespace, &[]).unwrap());
    fs::write(&home, b"- id: llm-pi-ai\n  config: {providers: {}}\n").unwrap();
    assert!(scanner.dsh_web_default_in_use(namespace, &[]).is_err());
}
