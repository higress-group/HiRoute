//! Isolate tests that compose production discovery without mutating process-global HOME.

mod listener_port;
pub(crate) use listener_port::ListenerPortLease;

pub(crate) fn isolated_agent_home(test_name: &str) -> bool {
    isolated_agent_home_inner(test_name, false, None)
}
pub(crate) fn isolated_agent_home_with_path(test_name: &str, path: &std::ffi::OsStr) -> bool {
    isolated_agent_home_inner(test_name, false, Some(path))
}
pub(crate) fn isolated_ignored_agent_home(test_name: &str) -> bool {
    isolated_agent_home_inner(test_name, true, None)
}
fn isolated_agent_home_inner(
    test_name: &str,
    ignored: bool,
    path: Option<&std::ffi::OsStr>,
) -> bool {
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
    let directory = private_tempdir();
    let home = directory.path().canonicalize().unwrap();
    let claude_home = home.join(".claude");
    #[cfg(not(windows))]
    std::fs::create_dir(&claude_home).unwrap();
    #[cfg(windows)]
    drop(hiroute_platform::windows::security::create_owner_only_directory(&claude_home).unwrap());
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
        .env("CLAUDE_CONFIG_DIR", &claude_home);
    #[cfg(unix)]
    command.env(
        "PATH",
        path.unwrap_or(std::ffi::OsStr::new("/usr/bin:/bin")),
    );
    #[cfg(windows)]
    {
        let system =
            hiroute_platform::windows::launcher::system_executable(std::path::Path::new("cmd.exe"))
                .unwrap();
        command
            .env("USERPROFILE", &home)
            .env("APPDATA", home.join("AppData/Roaming"))
            .env("LOCALAPPDATA", home.join("AppData/Local"))
            .env("PATH", path.unwrap_or(system.parent().unwrap().as_os_str()));
    }
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
    ]
    .into_iter()
    .chain(
        hiroute_domain::CLAUDE_CONTEXT_CONFLICT_ENVIRONMENT
            .iter()
            .copied(),
    )
    // OpenCode selectors must follow this child's private HOME, rather than the
    // host agent's XDG roots, inline configuration or nonresumable database.
    .chain([
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_CACHE_HOME",
        "XDG_STATE_HOME",
        "OPENCODE_CONFIG",
        "OPENCODE_CONFIG_DIR",
        "OPENCODE_CONFIG_CONTENT",
        "OPENCODE_DB",
        "OPENCODE_TEST_MANAGED_CONFIG_DIR",
    ]) {
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
#[cfg(not(windows))]
pub(crate) type PrivateTempDir = tempfile::TempDir;

#[cfg(windows)]
pub(crate) struct PrivateTempDir {
    _cleanup: tempfile::TempDir,
    path: std::path::PathBuf,
}

#[cfg(windows)]
impl PrivateTempDir {
    pub(crate) fn path(&self) -> &std::path::Path {
        &self.path
    }
}

pub(crate) fn private_tempdir() -> PrivateTempDir {
    let builder = tempfile::Builder::new();
    #[cfg(unix)]
    let builder = {
        use std::os::unix::fs::PermissionsExt;
        let mut builder = builder;
        builder.permissions(std::fs::Permissions::from_mode(0o700));
        builder
    };
    let directory = builder.tempdir().unwrap();
    #[cfg(not(windows))]
    {
        directory
    }
    #[cfg(windows)]
    {
        let path = directory.path().join("private");
        drop(hiroute_platform::windows::security::create_owner_only_directory(&path).unwrap());
        PrivateTempDir {
            _cleanup: directory,
            path,
        }
    }
}
