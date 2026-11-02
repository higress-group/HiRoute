use super::*;

// Frozen output from the immediately preceding installed 0.5.0 development build.
const PREVIOUS: &[u8] = include_bytes!("fixtures/codex-worker-serial-546.json");

#[test]
fn codex_parallel_default_preserves_a_previous_serial_task_on_continue() {
    let fixture = private_fixture();
    let old: Value = serde_json::from_slice(PREVIOUS).unwrap();
    let alias = old["models"][0]["slug"].as_str().unwrap();
    let workspace = hiroute_domain::WorkspaceId::parse("workspace").unwrap();
    let session = TaskSessionRoot::prepare(
        fixture.path(),
        &workspace,
        "root",
        "task-serial",
        WorkerHarnessV1::CodexCli,
        SessionRootUse::New,
    )
    .unwrap();
    let path = session
        .prepare_codex_catalog(alias, PREVIOUS, false)
        .unwrap();
    let mut enabled = old.clone();
    enabled["models"][0]["supports_parallel_tool_calls"] = json!(true);
    let enabled = serde_json::to_vec(&enabled).unwrap();
    let transcript = session
        .path()
        .join("sessions/2026/10/14/rollout-serial-session.jsonl");
    #[cfg(windows)]
    hiroute_platform::windows::security::ensure_private_directory(transcript.parent().unwrap())
        .unwrap();
    #[cfg(not(windows))]
    std::fs::create_dir_all(transcript.parent().unwrap()).unwrap();
    hiroute_platform::durable::write_private_new(&transcript, b"original serial task history\n")
        .unwrap();
    let required = native_history(session.path(), WorkerHarnessV1::CodexCli, "serial-session")
        .unwrap()
        .into_iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    let continued = TaskSessionRoot::prepare(
        fixture.path(),
        &workspace,
        "root",
        "task-serial",
        WorkerHarnessV1::CodexCli,
        SessionRootUse::Continue {
            required_history: &required,
        },
    )
    .unwrap();
    let profile = try_request_at_with_catalog(
        WorkerHarnessV1::CodexCli,
        "synthetic-next-secret",
        alias,
        WorkerPermissionPolicyV1::ApproveAll,
        &fixture.path().join("run-next"),
        &continued,
        Some(&enabled),
    )
    .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), PREVIOUS);
    let config: Value = serde_json::from_str(profile.env["CODEX_CONFIG"].as_str()).unwrap();
    assert_eq!(config["model_catalog_json"], path.to_str().unwrap());
    // An explicit enable cannot masquerade as the omitted policy of an old task.
    assert_eq!(
        continued.prepare_codex_catalog(alias, &enabled, false),
        Err(DelegationErrorV1::Conflict)
    );
    // The only admitted drift is the previous false bit, not any other catalog data.
    for key in ["context_window", "display_name", "instructions"] {
        let mut drifted: Value = serde_json::from_slice(&enabled).unwrap();
        drifted["models"][0][key] = if key == "context_window" {
            json!(drifted["models"][0][key].as_u64().unwrap() - 1)
        } else {
            json!("changed")
        };
        assert_eq!(
            continued.prepare_codex_catalog(alias, &serde_json::to_vec(&drifted).unwrap(), true),
            Err(DelegationErrorV1::Conflict)
        );
    }
    assert_eq!(std::fs::read(&path).unwrap(), PREVIOUS);
    assert_eq!(
        std::fs::read(&transcript).unwrap(),
        b"original serial task history\n"
    );
    // New tasks get the current default; an explicit disable remains supported.
    for (task, bytes, expected) in [
        ("task-enabled", enabled.as_slice(), true),
        ("task-disabled", PREVIOUS, false),
    ] {
        let fresh = TaskSessionRoot::prepare(
            fixture.path(),
            &workspace,
            "root",
            task,
            WorkerHarnessV1::CodexCli,
            SessionRootUse::New,
        )
        .unwrap();
        let path = fresh.prepare_codex_catalog(alias, bytes, true).unwrap();
        let actual: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(
            actual["models"][0]["supports_parallel_tool_calls"],
            expected
        );
    }
}
