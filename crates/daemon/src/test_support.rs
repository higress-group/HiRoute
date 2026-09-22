//! Isolate tests that compose production discovery without mutating process-global HOME.

pub(crate) fn isolated_agent_home(test_name: &str) -> bool {
    isolated_agent_home_inner(test_name, false)
}
pub(crate) fn isolated_ignored_agent_home(test_name: &str) -> bool {
    isolated_agent_home_inner(test_name, true)
}
fn isolated_agent_home_inner(test_name: &str, ignored: bool) -> bool {
    const CHILD: &str = "HIROUTE_ISOLATED_DAEMON_TEST";
    if std::env::var(CHILD).as_deref() == Ok(test_name) {
        return false;
    }
    let executable = std::env::current_exe().unwrap();
    let selected = std::process::Command::new(&executable)
        .args(["--list", "--exact", test_name])
        .output()
        .unwrap();
    let expected = format!("{test_name}: test");
    assert!(
        selected.status.success()
            && String::from_utf8_lossy(&selected.stdout)
                .lines()
                .filter(|line| *line == expected)
                .count()
                == 1,
        "isolated daemon test must select exactly one case: {test_name}"
    );
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().canonicalize().unwrap();
    let claude_home = home.join(".claude");
    std::fs::create_dir(&claude_home).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::set_permissions(&claude_home, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    #[cfg(unix)]
    let mut command = {
        let mut command = std::process::Command::new("/bin/sh");
        command
            .args(["-c", "umask 077; exec \"$0\" \"$@\""])
            .arg(&executable);
        command
    };
    #[cfg(not(unix))]
    let mut command = std::process::Command::new(executable);
    if ignored {
        command.arg("--ignored");
    }
    command
        .args(["--exact", test_name, "--nocapture"])
        .env(CHILD, test_name)
        .env("HOME", &home)
        .env("CODEX_HOME", home.join(".codex"))
        .env("CLAUDE_CONFIG_DIR", &claude_home)
        .env("PATH", "/usr/bin:/bin");
    for name in [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_BASE_URL",
        "ANTHROPIC_CUSTOM_HEADERS",
        "ANTHROPIC_MODEL",
        "ANTHROPIC_DEFAULT_OPUS_MODEL",
        "ANTHROPIC_DEFAULT_SONNET_MODEL",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL",
        "ANTHROPIC_SMALL_FAST_MODEL",
        "CLAUDE_CODE_USE_BEDROCK",
        "CLAUDE_CODE_USE_FOUNDRY",
        "CLAUDE_CODE_USE_VERTEX",
    ] {
        command.env_remove(name);
    }
    let status = command.status().unwrap();
    assert!(status.success(), "isolated daemon test failed: {test_name}");
    true
}

#[test]
#[should_panic(expected = "isolated daemon test must select exactly one case")]
fn missing_isolated_child_cannot_report_success() {
    isolated_agent_home("missing::test::must_not_pass");
}

/// Filesystem-observing fixtures must be private regardless of the caller's umask.
pub(crate) fn private_tempdir() -> tempfile::TempDir {
    let mut builder = tempfile::Builder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(std::fs::Permissions::from_mode(0o700));
    }
    builder.tempdir().unwrap()
}
