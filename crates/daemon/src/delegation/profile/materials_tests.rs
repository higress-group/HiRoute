use super::super::materials::checked_run_root;
use super::*;
use hiroute_domain::delegation::{DelegationNativeRootStateV1, DelegationNativeRootV1};
use std::fs;

fn root(
    base: &Path,
    task: &str,
    harness: WorkerHarnessV1,
    usage: SessionRootUse<'_>,
) -> Result<TaskSessionRoot, DelegationErrorV1> {
    TaskSessionRoot::prepare(
        base,
        &hiroute_domain::WorkspaceId::parse("workspace").unwrap(),
        "root",
        task,
        harness,
        usage,
    )
}

fn profile(
    harness: WorkerHarnessV1,
    token: &str,
    private: &Path,
    session: &TaskSessionRoot,
) -> CandidateWorkerProfile {
    try_request_at(
        harness,
        token,
        "alias",
        WorkerPermissionPolicyV1::ApproveAll,
        private,
        session,
    )
    .unwrap()
}

#[test]
fn codex_startup_provider_is_private_token_free_and_never_overwritten() {
    let fixture = private_fixture();
    let session = root(
        fixture.path(),
        "task",
        WorkerHarnessV1::CodexCli,
        SessionRootUse::New,
    )
    .unwrap();
    let gateway = "127.0.0.1:12345".parse().unwrap();
    session.prepare_codex_provider(gateway, None).unwrap();
    let path = session.path().join("config.toml");
    let before = fs::read_to_string(&path).unwrap();
    assert!(before.contains("requires_openai_auth = false"));
    assert!(before.contains("env_key = \"HIROUTE_RUN_TOKEN\""));
    session.prepare_codex_provider(gateway, None).unwrap();
    assert_eq!(
        session.prepare_codex_provider("127.0.0.1:12346".parse().unwrap(), None),
        Err(DelegationErrorV1::Conflict)
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), before);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

// Synthetic history proves material lifetime only, not native adapter load behavior.
#[test]
fn continue_changes_run_root_and_token_but_keeps_exact_history_after_run_cleanup() {
    for harness in [WorkerHarnessV1::CodexCli, WorkerHarnessV1::ClaudeCode] {
        let fixture = private_fixture();
        let daily = fixture.path().join("daily-config");
        fs::write(&daily, "daily-secret").unwrap();
        let first_root = root(fixture.path(), "task", harness, SessionRootUse::New).unwrap();
        let first = profile(
            harness,
            "old-run-token",
            &fixture.path().join("run-old"),
            &first_root,
        );
        assert_eq!(
            first.materials.directories,
            vec![PathBuf::from("home"), PathBuf::from("tmp")]
        );
        assert!(first.materials.files.is_empty());
        assert!(!first.private_root.exists()); // profile does not own launcher resources
        fs::create_dir(&first.private_root).unwrap();
        for relative in &first.materials.directories {
            fs::create_dir(first.private_root.join(relative)).unwrap();
        }
        let history = PathBuf::from("exact-native-session.jsonl");
        fs::write(
            first_root.path().join(&history),
            "synthetic original history",
        )
        .unwrap();
        // Stand-in for 18's cleanup of only its own run directory.
        fs::remove_dir_all(&first.private_root).unwrap();
        let continued = root(
            fixture.path(),
            "task",
            harness,
            SessionRootUse::Continue {
                required_history: std::slice::from_ref(&history),
            },
        )
        .unwrap();
        let next = profile(
            harness,
            "new-run-token",
            &fixture.path().join("run-next"),
            &continued,
        );
        assert_ne!(first.private_root, next.private_root);
        assert_eq!(first.session_root, next.session_root);
        let native_env = if harness == WorkerHarnessV1::CodexCli {
            "CODEX_HOME"
        } else {
            "CLAUDE_CONFIG_DIR"
        };
        assert_eq!(
            next.env[native_env].as_str(),
            continued.path().to_str().unwrap()
        );
        assert_eq!(
            next.env["HOME"].as_str(),
            next.private_root.join("home").to_str().unwrap()
        );
        assert_eq!(
            next.env["TMPDIR"].as_str(),
            next.private_root.join("tmp").to_str().unwrap()
        );
        assert!(
            next.env
                .values()
                .all(|v| !v.contains("old-run-token") && !v.contains("daily-secret"))
        );
        assert!(next.env.values().any(|v| v.as_str() == "new-run-token"));
        assert_eq!(
            fs::read_to_string(continued.path().join(history)).unwrap(),
            "synthetic original history"
        );
        assert_eq!(
            fs::read_dir(continued.path()).unwrap().count(),
            if harness == WorkerHarnessV1::CodexCli {
                2
            } else {
                1
            }
        );
        if harness == WorkerHarnessV1::CodexCli {
            let config = fs::read_to_string(continued.path().join("config.toml")).unwrap();
            assert!(config.contains("env_key = \"HIROUTE_RUN_TOKEN\""));
            assert!(!config.contains("old-run-token") && !config.contains("new-run-token"));
        }
        assert_eq!(fs::read_to_string(daily).unwrap(), "daily-secret");
    }
}

#[test]
fn roots_are_scoped_to_task_harness_and_controlled_workspace() {
    let fixture = private_fixture();
    let a = root(
        fixture.path(),
        "a",
        WorkerHarnessV1::CodexCli,
        SessionRootUse::New,
    )
    .unwrap();
    let b = root(
        fixture.path(),
        "b",
        WorkerHarnessV1::CodexCli,
        SessionRootUse::New,
    )
    .unwrap();
    let c = root(
        fixture.path(),
        "a",
        WorkerHarnessV1::ClaudeCode,
        SessionRootUse::New,
    )
    .unwrap();
    let d = TaskSessionRoot::prepare(
        fixture.path(),
        &hiroute_domain::WorkspaceId::parse("workspace").unwrap(),
        "other-root",
        "a",
        WorkerHarnessV1::CodexCli,
        SessionRootUse::New,
    )
    .unwrap();
    let e = TaskSessionRoot::prepare(
        fixture.path(),
        &hiroute_domain::WorkspaceId::parse("other-workspace").unwrap(),
        "root",
        "a",
        WorkerHarnessV1::CodexCli,
        SessionRootUse::New,
    )
    .unwrap();
    let paths = [a.path(), b.path(), c.path(), d.path(), e.path()];
    for (index, path) in paths.iter().enumerate() {
        assert!(!paths[..index].contains(path));
    }
    assert_eq!(
        a.check_binding(WorkerHarnessV1::ClaudeCode, "root"),
        Err(DelegationErrorV1::InvalidArguments)
    );
    assert_eq!(
        a.check_binding(WorkerHarnessV1::CodexCli, "other-root"),
        Err(DelegationErrorV1::InvalidArguments)
    );
    assert!(matches!(
        root(
            fixture.path(),
            "a",
            WorkerHarnessV1::CodexCli,
            SessionRootUse::New
        ),
        Err(DelegationErrorV1::Conflict)
    ));
}

#[test]
fn native_root_marker_binds_the_created_identity_and_is_bounded() {
    let fixture = private_fixture();
    let session = root(
        fixture.path(),
        "marker-task",
        WorkerHarnessV1::CodexCli,
        SessionRootUse::New,
    )
    .unwrap();
    let mut record = DelegationNativeRootV1 {
        workspace_id: hiroute_domain::WorkspaceId::parse("workspace").unwrap(),
        task_id: "marker-task".into(),
        root_generation: 1,
        harness: WorkerHarnessV1::CodexCli,
        workspace_root_identity: "root".into(),
        relative_root: session.path().file_name().unwrap().to_str().unwrap().into(),
        creation_nonce: "marker-creation".into(),
        state: DelegationNativeRootStateV1::Creating,
        use_revision: 1,
        managed_base_path: None,
        filesystem_identity: None,
        cleanup_claim: None,
        deletion_batches: 0,
        last_cleanup_failure: None,
    };
    let identity = session
        .create_ownership_marker(fixture.path(), &record)
        .unwrap();
    record.state = DelegationNativeRootStateV1::Ready;
    record.managed_base_path = Some(
        fs::canonicalize(fixture.path())
            .unwrap()
            .to_str()
            .unwrap()
            .into(),
    );
    record.filesystem_identity = Some(identity.clone());
    assert_eq!(
        session.verify_ownership(fixture.path(), &record).unwrap(),
        identity
    );

    fs::write(
        session.path().join(".hiroute-native-root-v1.json"),
        vec![b'x'; 4097],
    )
    .unwrap();
    assert_eq!(
        session.verify_ownership(fixture.path(), &record),
        Err(DelegationErrorV1::ResumeUnavailable)
    );
}

#[test]
fn missing_or_invalid_history_fails_without_creating_a_replacement() {
    let fixture = private_fixture();
    let harness = WorkerHarnessV1::CodexCli;
    let session = root(fixture.path(), "a", harness, SessionRootUse::New).unwrap();
    for paths in [
        vec![],
        vec![PathBuf::from("missing")],
        vec![PathBuf::from("../outside")],
        vec![PathBuf::from("/")],
    ] {
        assert!(matches!(
            root(
                fixture.path(),
                "a",
                harness,
                SessionRootUse::Continue {
                    required_history: &paths
                }
            ),
            Err(DelegationErrorV1::ResumeUnavailable)
        ));
    }
    fs::remove_dir(session.path()).unwrap();
    assert!(matches!(
        root(
            fixture.path(),
            "a",
            harness,
            SessionRootUse::Continue {
                required_history: &["history".into()]
            }
        ),
        Err(DelegationErrorV1::ResumeUnavailable)
    ));
    assert!(!session.path().exists());
}

#[test]
fn existing_equal_nested_roots_are_rejected_without_touching_them() {
    let fixture = private_fixture();
    let session = root(
        fixture.path(),
        "a",
        WorkerHarnessV1::CodexCli,
        SessionRootUse::New,
    )
    .unwrap();
    assert!(checked_run_root(session.path(), &session).is_err());
    assert!(checked_run_root(&session.path().join("run"), &session).is_err());
    assert!(checked_run_root(fixture.path(), &session).is_err());
    let existing = fixture.path().join("existing");
    fs::create_dir(&existing).unwrap();
    fs::write(existing.join("keep"), "untouched").unwrap();
    assert_eq!(
        checked_run_root(&existing, &session),
        Err(DelegationErrorV1::Conflict)
    );
    assert_eq!(
        fs::read_to_string(existing.join("keep")).unwrap(),
        "untouched"
    );
    let similarly_named = fixture.path().join(format!(
        "{}-sibling",
        session.path().file_name().unwrap().to_str().unwrap()
    ));
    assert!(checked_run_root(&similarly_named, &session).is_ok());
}

#[cfg(unix)]
#[test]
fn filesystem_aliases_and_history_links_cannot_bypass_checks() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let fixture = private_fixture();
    let session = root(
        fixture.path(),
        "a",
        WorkerHarnessV1::CodexCli,
        SessionRootUse::New,
    )
    .unwrap();
    let alias = fixture.path().join("alias");
    symlink(session.path(), &alias).unwrap();
    assert!(checked_run_root(&alias.join("run"), &session).is_err());
    let nested = session.path().join("nested");
    fs::create_dir(&nested).unwrap();
    fs::set_permissions(&nested, fs::Permissions::from_mode(0o700)).unwrap();
    // Final parent is a real directory, but an ancestor alias resolves inside session_root.
    assert!(checked_run_root(&alias.join("nested/run"), &session).is_err());
    fs::write(fixture.path().join("outside"), "outside").unwrap();
    symlink(
        fixture.path().join("outside"),
        session.path().join("history"),
    )
    .unwrap();
    assert!(matches!(
        root(
            fixture.path(),
            "a",
            WorkerHarnessV1::CodexCli,
            SessionRootUse::Continue {
                required_history: &["history".into()]
            }
        ),
        Err(DelegationErrorV1::ResumeUnavailable)
    ));
    fs::set_permissions(session.path(), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(checked_run_root(&fixture.path().join("run"), &session).is_err());
}
