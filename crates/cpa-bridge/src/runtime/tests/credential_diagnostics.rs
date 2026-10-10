use super::*;

#[test]
fn credential_inspection_reports_closed_causes_without_input_or_paths() {
    for (case, expected) in [
        ("missing", "credential_source_missing"),
        ("mode", "credential_login_unsupported"),
        ("account", "credential_account_missing"),
        ("store", "credential_store_unsupported"),
        ("invalid", "credential_invalid"),
    ] {
        let root = tempfile::tempdir().unwrap();
        let logs = tempfile::tempdir().unwrap();
        let diagnostics_root = logs.path().join("d");
        let report = DiagnosticRuntime::start(RuntimeConfig {
            root: diagnostics_root.clone(),
            role: hiroute_diagnostics::event::ProcessRole::Daemon,
            component: hiroute_diagnostics::record::Component::Cpa,
            parent_session_id: None,
            level_override: Some(hiroute_diagnostics::level::DiagnosticLevel::Debug),
        });
        let mut runtime = fixture_runtime(
            &root,
            Arc::new(FakeBackend::default()),
            Arc::new(FakeControl::default()),
            2,
        )
        .with_diagnostics(report.port());
        let path = root.path().join("credential-path-sentinel.json");
        let mut auth = json!({"auth_mode":"chatgpt", "tokens":{"access_token":"credential-access-sentinel", "account_id":"credential-account-sentinel"}, "refresh_token":"credential-refresh-sentinel"});
        match case {
            "mode" => auth["auth_mode"] = json!("apikey"),
            "account" => {
                auth["tokens"].as_object_mut().unwrap().remove("account_id");
            }
            "invalid" => auth["tokens"]["access_token"] = json!(""),
            _ => {}
        }
        if case != "missing" {
            std::fs::write(&path, serde_json::to_vec(&auth).unwrap()).unwrap();
        }
        let mut spec = BorrowedCodexAuthSpec::new(path);
        if case == "store" {
            let config = root.path().join("config-path-sentinel.toml");
            std::fs::write(&config, "cli_auth_credentials_store = \"keyring\"\n").unwrap();
            spec = spec.with_store_config(Some(config));
        }
        runtime.spec.borrowed_codex_auth = Some(spec);
        assert!(runtime.inspect_subscription_for_check().is_err());
        report.shutdown();
        let log = std::fs::read_to_string(diagnostics_root.join("daemon/current.jsonl")).unwrap();
        let stages = stage_records(&log);
        assert_eq!(stages.len(), 2);
        assert_eq!(stages[0].stage, "native_credential_read");
        assert_eq!(stages[0].outcome, "\"entered\"");
        assert!(
            stages[1].outcome.contains(expected),
            "missing {expected}: {stages:?}"
        );
        assert!(!log.contains("sentinel"), "credential or path leaked");
        assert!(
            !log.contains(&root.path().display().to_string()),
            "source root leaked"
        );
    }
}
