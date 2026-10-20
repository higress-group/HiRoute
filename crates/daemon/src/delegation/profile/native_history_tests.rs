use super::*;

const SESSION: &str = "71a2b020-312e-49cd-b084-924cbb3642ad";

#[cfg(unix)]
#[test]
fn opencode_review_rejects_unprojectable_xdg_roots_for_new_and_continue() {
    use std::os::unix::fs::symlink;
    for index in 0..4 {
        let storage = crate::test_support::private_tempdir();
        let home = storage.path().canonicalize().unwrap().join("home");
        let workspace = storage.path().canonicalize().unwrap().join("workspace");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&workspace).unwrap();
        let mut native =
            hiroute_integrations::OpenCodeNativeContext::from_lookup(&home, |_| None).unwrap();
        let selected = [
            &mut native.config_root,
            &mut native.data_root,
            &mut native.cache_root,
            &mut native.state_root,
        ]
        .into_iter()
        .nth(index)
        .unwrap();
        let borrowed = home.join("borrowed-root");
        fs::create_dir_all(&borrowed).unwrap();
        fs::create_dir_all(selected.parent().unwrap()).unwrap();
        symlink(&borrowed, &*selected).unwrap();
        let session = TaskSessionRoot::prepare(
            storage.path(),
            &WorkspaceId::default(),
            "root",
            "task",
            WorkerHarnessV1::OpenCode,
            SessionRootUse::New,
        )
        .unwrap();
        assert!(
            session.bind_opencode_context(&native, &workspace).is_err(),
            "XDG root {index}"
        );

        // A descriptor produced by the old binding code must also fail before a Continue launch.
        *[
            &mut native.config_root,
            &mut native.data_root,
            &mut native.cache_root,
            &mut native.state_root,
        ][index] = borrowed.clone();
        if index == 1 {
            native.database = borrowed.join("opencode.db");
        }
        write_owned_json(
            session.path(),
            CONTEXT_FILE,
            &ContextDescriptor {
                version: 2,
                harness: WorkerHarnessV1::OpenCode,
                context: BorrowedNativeContext {
                    home: home.clone(),
                    config_root: native.config_root.clone(),
                    workspace,
                    opencode: Some(native),
                },
            },
        )
        .unwrap();
        assert!(
            session.borrowed_context().is_err(),
            "Continue XDG root {index}"
        );
        assert!(borrowed.is_dir());
    }
}

#[cfg(unix)]
#[test]
fn opencode_xdg_directory_links_preserve_the_admitted_child_roots() {
    use std::os::unix::fs::symlink;
    let storage = crate::test_support::private_tempdir();
    let base = storage.path().canonicalize().unwrap();
    let home = base.join("home");
    let workspace = base.join("workspace");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&workspace).unwrap();
    let native = hiroute_integrations::OpenCodeNativeContext::from_lookup(&home, |_| None).unwrap();
    for (index, path) in [
        &native.config_root,
        &native.data_root,
        &native.cache_root,
        &native.state_root,
    ]
    .into_iter()
    .enumerate()
    {
        let actual = base.join(format!("selected-{index}/opencode"));
        fs::create_dir_all(&actual).unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        symlink(&actual, path).unwrap();
    }
    let session = TaskSessionRoot::prepare(
        &base,
        &WorkspaceId::default(),
        "root",
        "task",
        WorkerHarnessV1::OpenCode,
        SessionRootUse::New,
    )
    .unwrap();
    session.bind_opencode_context(&native, &workspace).unwrap();
    let bound = session
        .borrowed_context()
        .unwrap()
        .unwrap()
        .opencode
        .unwrap();
    for (source, admitted) in [
        (&native.config_root, &bound.config_root),
        (&native.data_root, &bound.data_root),
        (&native.cache_root, &bound.cache_root),
        (&native.state_root, &bound.state_root),
    ] {
        assert_eq!(&fs::canonicalize(source).unwrap(), admitted);
        assert_eq!(admitted.parent().unwrap().join("opencode"), *admitted);
    }
}

fn canonical(path: &Path) -> PathBuf {
    super::super::super::native_context::native_path(&fs::canonicalize(path).unwrap())
}

struct Fixture {
    _storage: tempfile::TempDir,
    base: PathBuf,
    home: PathBuf,
    config: PathBuf,
    workspace: PathBuf,
    session: TaskSessionRoot,
}

impl Fixture {
    fn new(harness: WorkerHarnessV1) -> Self {
        let storage = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(storage.path(), fs::Permissions::from_mode(0o700)).unwrap();
        }
        #[cfg(windows)]
        let base = {
            let base = storage.path().join("private");
            hiroute_platform::windows::security::create_owner_only_directory(&base).unwrap();
            base
        };
        #[cfg(not(windows))]
        let base = storage.path().to_owned();
        let home = base.join("daily-home");
        let config = home.join("native-config");
        let workspace = base.join("workspace");
        fs::create_dir_all(&config).unwrap();
        fs::create_dir(&workspace).unwrap();
        let home = canonical(&home);
        let config = canonical(&config);
        let workspace = canonical(&workspace);
        let session = TaskSessionRoot::prepare(
            &base,
            &WorkspaceId::parse("workspace").unwrap(),
            "workspace-root",
            "task",
            harness,
            SessionRootUse::New,
        )
        .unwrap();
        session
            .bind_borrowed_context(&home, &config, &workspace)
            .unwrap();
        Self {
            _storage: storage,
            base,
            home,
            config,
            workspace,
            session,
        }
    }

    fn reopen(&self, required: &[PathBuf]) -> Result<TaskSessionRoot, DelegationErrorV1> {
        TaskSessionRoot::prepare(
            &self.base,
            &WorkspaceId::parse("workspace").unwrap(),
            "workspace-root",
            "task",
            self.session.harness,
            SessionRootUse::Continue {
                required_history: required,
            },
        )
    }

    fn claude_history(&self) -> PathBuf {
        // Filesystem fixtures use the short-path native spelling. Unicode and long-path
        // contract cases below use literal expected values captured from the native algorithm.
        let cwd = canonical(&self.workspace);
        let project = cwd
            .to_str()
            .unwrap()
            .replace(|ch: char| !ch.is_ascii_alphanumeric(), "-");
        self.config
            .join("projects")
            .join(project)
            .join(format!("{SESSION}.jsonl"))
    }
}

#[test]
fn borrowed_codex_and_qoder_retain_exact_binding_without_claiming_native_history_availability() {
    for harness in [WorkerHarnessV1::CodexCli, WorkerHarnessV1::QoderCli] {
        let fixture = Fixture::new(harness);
        let daily = fixture.config.join("daily-history.jsonl");
        fs::write(&daily, b"unrelated user's history").unwrap();
        let before = fs::read(&daily).unwrap();
        let materials = native_history(fixture.session.path(), harness, SESSION).unwrap();
        assert_eq!(materials, [SESSION_FILE]);
        assert!(!fixture.config.join("sessions").exists());
        assert!(!fixture.session.path().join("sessions").exists());
        let required = materials.iter().map(PathBuf::from).collect::<Vec<_>>();
        let resumed = fixture.reopen(&required).unwrap();
        resumed.verify_native_session(SESSION).unwrap();
        assert_eq!(
            resumed.verify_native_session("other-session"),
            Err(DelegationErrorV1::ResumeUnavailable)
        );
        assert_eq!(
            TaskSessionRoot::native_transcript_path(fixture.session.path(), harness, SESSION),
            Err(DelegationErrorV1::ResumeUnavailable)
        );
        assert_eq!(fs::read(daily).unwrap(), before);
    }
}

#[test]
fn borrowed_context_survives_restart_and_cannot_be_rebound_by_continue() {
    let fixture = Fixture::new(WorkerHarnessV1::CodexCli);
    let context = fixture.session.borrowed_context().unwrap().unwrap();
    assert_eq!(context.home, canonical(&fixture.home));
    assert_eq!(context.config_root, canonical(&fixture.config));
    assert_eq!(context.workspace, canonical(&fixture.workspace));
    fixture
        .session
        .bind_borrowed_context(&fixture.home, &fixture.config, &fixture.workspace)
        .unwrap();
    assert_eq!(
        fixture
            .session
            .bind_borrowed_context(&fixture.home, &fixture.home, &fixture.workspace),
        Err(DelegationErrorV1::Conflict)
    );
    native_history(fixture.session.path(), WorkerHarnessV1::CodexCli, SESSION).unwrap();
    let resumed = fixture.reopen(&[PathBuf::from(SESSION_FILE)]).unwrap();
    assert_eq!(resumed.borrowed_context().unwrap(), Some(context));
    assert_eq!(
        resumed.bind_borrowed_context(&fixture.home, &fixture.config, &fixture.workspace),
        Err(DelegationErrorV1::Conflict)
    );
}

#[test]
fn borrowed_claude_checks_only_its_workspace_and_exact_native_session() {
    let fixture = Fixture::new(WorkerHarnessV1::ClaudeCode);
    let history = fixture.claude_history();
    fs::create_dir_all(history.parent().unwrap()).unwrap();
    fs::write(&history, b"opaque native transcript\n").unwrap();
    // More than the old scanner's bound, plus an identical ID in a different project.
    let unrelated = fixture.config.join("projects/unrelated");
    fs::create_dir(&unrelated).unwrap();
    for index in 0..520 {
        fs::write(unrelated.join(format!("{index}.jsonl")), b"other").unwrap();
    }
    fs::write(
        unrelated.join(format!("{SESSION}.jsonl")),
        b"other workspace",
    )
    .unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("absent", unrelated.join("ignored-link")).unwrap();
    assert_eq!(
        TaskSessionRoot::native_transcript_path(
            fixture.session.path(),
            WorkerHarnessV1::ClaudeCode,
            SESSION
        )
        .unwrap(),
        history
    );
    native_history(fixture.session.path(), WorkerHarnessV1::ClaudeCode, SESSION).unwrap();
    let resumed = fixture.reopen(&[PathBuf::from(SESSION_FILE)]).unwrap();
    resumed.verify_native_session(SESSION).unwrap();
    fs::remove_file(&history).unwrap();
    assert_eq!(
        resumed.verify_native_session(SESSION),
        Err(DelegationErrorV1::ResumeUnavailable)
    );
    assert!(matches!(
        fixture.reopen(&[PathBuf::from(SESSION_FILE)]),
        Err(DelegationErrorV1::ResumeUnavailable)
    ));
    assert!(unrelated.join(format!("{SESSION}.jsonl")).exists());
}

#[test]
fn missing_or_replaced_descriptor_never_becomes_legacy_continuation() {
    let fixture = Fixture::new(WorkerHarnessV1::CodexCli);
    native_history(fixture.session.path(), WorkerHarnessV1::CodexCli, SESSION).unwrap();
    let descriptor = fixture.session.path().join(CONTEXT_FILE);
    let original = fs::read(&descriptor).unwrap();
    fs::write(&descriptor, b"{}").unwrap();
    assert!(fixture.session.borrowed_context().is_err());
    assert!(fixture.reopen(&[PathBuf::from(SESSION_FILE)]).is_err());
    fs::write(&descriptor, &original).unwrap();
    let mut changed: serde_json::Value = serde_json::from_slice(&original).unwrap();
    changed["context"]["config_root"] = serde_json::json!(fixture.home);
    fs::write(&descriptor, serde_json::to_vec(&changed).unwrap()).unwrap();
    assert!(fixture.reopen(&[PathBuf::from(SESSION_FILE)]).is_err());
    fs::remove_file(&descriptor).unwrap();
    assert!(fixture.session.borrowed_context().is_err());
}

#[test]
fn native_session_binding_is_immutable_and_is_not_an_arbitrary_material_path() {
    let fixture = Fixture::new(WorkerHarnessV1::CodexCli);
    native_history(fixture.session.path(), WorkerHarnessV1::CodexCli, SESSION).unwrap();
    assert_eq!(
        native_history(
            fixture.session.path(),
            WorkerHarnessV1::CodexCli,
            "another-native-session"
        ),
        Err(DelegationErrorV1::Conflict)
    );
    fs::write(fixture.session.path().join("unrelated.jsonl"), b"unrelated").unwrap();
    assert!(fixture.reopen(&[PathBuf::from("unrelated.jsonl")]).is_err());
    for invalid in [
        "",
        "../session",
        "session/name",
        "session\\name",
        "session\n",
    ] {
        assert!(
            native_history(fixture.session.path(), WorkerHarnessV1::CodexCli, invalid).is_err()
        );
    }
}

#[cfg(unix)]
#[test]
fn owned_metadata_and_exact_claude_transcripts_reject_links() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new(WorkerHarnessV1::ClaudeCode);
    let history = fixture.claude_history();
    fs::create_dir_all(history.parent().unwrap()).unwrap();
    let target = fixture.home.join("not-this-session");
    fs::write(&target, b"unrelated").unwrap();
    symlink(&target, &history).unwrap();
    assert!(native_history(fixture.session.path(), WorkerHarnessV1::ClaudeCode, SESSION).is_err());
    let descriptor = fixture.session.path().join(CONTEXT_FILE);
    let bytes = fs::read(&descriptor).unwrap();
    fs::remove_file(&descriptor).unwrap();
    let other = fixture.session.path().join("other-descriptor.json");
    fs::write(&other, bytes).unwrap();
    symlink(&other, descriptor).unwrap();
    assert!(fixture.session.borrowed_context().is_err());
    assert_eq!(fs::read(target).unwrap(), b"unrelated");
}

#[test]
fn claude_workspace_keys_match_native_unicode_and_long_path_vectors() {
    assert_eq!(
        claude_workspace_key(Path::new("/Users/test/cafe\u{301}/任务/😀")).unwrap(),
        "-Users-test-caf-------"
    );
    assert_eq!(
        claude_workspace_key(Path::new("/Users/test/café/任务/😀")).unwrap(),
        "-Users-test-caf-------"
    );
    assert_eq!(
        claude_workspace_key(Path::new(&format!("/tmp/{}", "abc".repeat(80)))).unwrap(),
        format!("-tmp-{}-89qna1", "abc".repeat(65))
    );
    assert_eq!(
        claude_workspace_key(Path::new(&format!("/tmp/{}", "😀".repeat(110)))).unwrap(),
        format!("-tmp{}-na1or3", "-".repeat(196))
    );
}

#[test]
fn creating_the_descriptor_does_not_create_a_missing_native_config_root() {
    let fixture = Fixture::new(WorkerHarnessV1::CodexCli);
    // Use a distinct new task: published descriptors are never overwritten.
    let session = TaskSessionRoot::prepare(
        &fixture.base,
        &WorkspaceId::parse("workspace").unwrap(),
        "workspace-root",
        "another-task",
        WorkerHarnessV1::CodexCli,
        SessionRootUse::New,
    )
    .unwrap();
    let absent = fixture.home.join("new-client/config");
    session
        .bind_borrowed_context(&fixture.home, &absent, &fixture.workspace)
        .unwrap();
    assert!(!absent.exists());
    assert_eq!(
        session.borrowed_context().unwrap().unwrap().config_root,
        absent
    );
}

#[test]
fn qoder_missing_borrowed_descriptor_cannot_be_reinterpreted_as_private_legacy_history() {
    let fixture = Fixture::new(WorkerHarnessV1::QoderCli);
    fs::remove_file(fixture.session.path().join(CONTEXT_FILE)).unwrap();
    assert!(matches!(
        fixture.session.borrowed_context(),
        Err(DelegationErrorV1::ResumeUnavailable)
    ));
    assert!(matches!(
        native_history(fixture.session.path(), WorkerHarnessV1::QoderCli, SESSION),
        Err(DelegationErrorV1::ResumeUnavailable)
    ));
}
