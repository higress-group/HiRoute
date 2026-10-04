use super::*;

fn session(root: &Path, task: &str) -> TaskSessionRoot {
    TaskSessionRoot::prepare(
        root,
        &hiroute_domain::WorkspaceId::default(),
        "root",
        task,
        WorkerHarnessV1::QoderCli,
        SessionRootUse::New,
    )
    .unwrap()
}

fn request(
    root: &Path,
    session: &TaskSessionRoot,
    context: &NativeWorkerContext,
    token: &str,
    policy: WorkerPermissionPolicyV1,
) -> Result<CandidateWorkerProfile, DelegationErrorV1> {
    try_request_in_context(
        WorkerHarnessV1::QoderCli,
        token,
        "plan/branch:cheap",
        policy,
        root,
        session,
        None,
        context,
        None,
    )
}

#[test]
fn native_qoder_uses_selected_cli_shared_resources_and_only_transient_managed_route() {
    let fixture = private_fixture();
    let home = fixture.path().join("daily home");
    let config = home.join("selected-qoder");
    std::fs::create_dir_all(&config).unwrap();
    let daily_settings = config.join("settings.json");
    let before = br#"{"model":{"name":"daily/foreign"},"providers":{"daily":{"baseUrl":"https://daily.invalid"}}}"#;
    std::fs::write(&daily_settings, before).unwrap();
    let context = NativeWorkerContext::borrowed(&home, &config).unwrap();
    let session = session(fixture.path(), "task");
    let profile = request(
        &fixture.path().join("run"),
        &session,
        &context,
        "first-run-secret",
        WorkerPermissionPolicyV1::ApproveAll,
    )
    .unwrap();
    let next = request(
        &fixture.path().join("continued"),
        &session,
        &context,
        "next-run-secret",
        WorkerPermissionPolicyV1::ApproveAll,
    )
    .unwrap();
    let another_session = self::session(fixture.path(), "neighbor");
    let neighbor = request(
        &fixture.path().join("neighbor-run"),
        &another_session,
        &context,
        "neighbor-secret",
        WorkerPermissionPolicyV1::ApproveAll,
    )
    .unwrap();
    assert_eq!(profile.executable, Path::new("/trusted/harness"));
    assert_eq!(profile.env["HOME"].as_str(), home.to_str().unwrap());
    assert_eq!(
        profile.env["QODER_CONFIG_DIR"].as_str(),
        config.to_str().unwrap()
    );
    assert!(!profile.env.contains_key("QODER_PERSONAL_ACCESS_TOKEN"));
    assert!(!profile.env.contains_key("NODE_OPTIONS"));
    assert_eq!(profile.native_session_mode(), "yolo");
    assert_eq!(
        profile.identity_contract,
        AcpNativeIdentityContract::QoderSessionV1
    );
    assert!(profile.codex_initialization_root().is_none());
    assert!(profile.session_meta.is_empty());
    assert_eq!(
        profile.native_selected_model_id(),
        next.native_selected_model_id()
    );
    assert_ne!(
        profile.native_selected_model_id(),
        neighbor.native_selected_model_id()
    );
    assert!(
        profile
            .native_selected_model_id()
            .ends_with("/plan/branch:cheap")
    );
    assert_eq!(
        profile.env["QODER_MODEL"].as_str(),
        profile.native_selected_model_id()
    );
    assert_eq!(next.env["HIROUTE_RUN_TOKEN"].as_str(), "next-run-secret");
    assert!(
        next.env
            .values()
            .all(|value| !value.contains("first-run-secret"))
    );
    assert_eq!(profile.materials.directories, [PathBuf::from("tmp")]);
    assert_eq!(profile.materials.files.len(), 1);
    let material = &profile.materials.files[0];
    assert!(!material.executable);
    assert!(!String::from_utf8_lossy(&material.contents).contains("first-run-secret"));
    let settings: Value = serde_json::from_slice(&material.contents).unwrap();
    let providers = settings["providers"].as_object().unwrap();
    assert_eq!(providers.len(), 1);
    let (provider_id, provider) = providers.iter().next().unwrap();
    assert_eq!(
        profile.native_selected_model_id(),
        format!("{provider_id}/plan/branch:cheap")
    );
    assert_eq!(provider["model"], "plan/branch:cheap");
    assert_eq!(provider["baseUrl"], "http://127.0.0.1:44123/v1");
    assert_eq!(provider["apiKey"], "${HIROUTE_RUN_TOKEN}");
    assert_eq!(provider["models"][0]["contextWindow"], 272_000);
    assert_eq!(provider["models"][0]["maxOutputTokens"], 4096);
    let continued: Value = serde_json::from_slice(&next.materials.files[0].contents).unwrap();
    assert_eq!(
        continued["providers"][provider_id]["models"],
        provider["models"]
    );
    let args = profile
        .args
        .iter()
        .map(|value| value.to_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(args[0], "--acp");
    for (flag, value) in [
        ("--config-dir", config.to_str().unwrap()),
        ("--setting-sources", "user,project,local"),
        ("--model", profile.native_selected_model_id()),
        ("--permission-mode", "yolo"),
        ("--tools", "Read,Write,Edit,Bash,Grep,Glob,Skill"),
        ("--mcp-config", r#"{"mcpServers":{}}"#),
    ] {
        assert!(args.windows(2).any(|pair| pair == [flag, value]));
    }
    assert!(args.contains(&"--strict-mcp-config"));
    assert!(args.windows(2).any(|pair| pair[0] == "--settings"
        && Path::new(pair[1]) == profile.private_root.join(&material.relative_path)));
    assert!(args.iter().all(|arg| !arg.contains("first-run-secret")));
    assert_eq!(std::fs::read(daily_settings).unwrap(), before);
}

#[test]
fn qoder_does_not_upgrade_unproven_restricted_policies_or_accept_an_adapter_runtime() {
    let fixture = private_fixture();
    let session = session(fixture.path(), "task");
    let context =
        NativeWorkerContext::borrowed(Path::new("/chosen-home"), Path::new("/chosen-config"))
            .unwrap();
    for policy in [
        WorkerPermissionPolicyV1::ApproveReads,
        WorkerPermissionPolicyV1::DenyAll,
    ] {
        let private = fixture.path().join("run");
        assert!(matches!(
            request(&private, &session, &context, "secret", policy),
            Err(DelegationErrorV1::CapabilityUnavailable)
        ));
        assert!(!private.exists());
    }
    assert!(matches!(
        try_request_in_context(
            WorkerHarnessV1::QoderCli,
            "secret",
            "plan/branch:cheap",
            WorkerPermissionPolicyV1::ApproveAll,
            &fixture.path().join("run"),
            &session,
            None,
            &context,
            Some(Path::new("/selected-node"))
        ),
        Err(DelegationErrorV1::InvalidArguments)
    ));
}
