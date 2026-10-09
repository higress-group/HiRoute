#![cfg(unix)]
use super::*;
use std::os::unix::fs::PermissionsExt;

fn program(root: &Path, text: &str) -> PathBuf {
    let path = root.join("agent");
    std::fs::write(&path, format!("#!/bin/sh\n{text}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    path
}

#[test]
fn agent_probe_missing_and_failed_probe_are_distinct() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        probe(&dir.path().join("missing"), Duration::from_millis(100)),
        ExecutableProbe::NotFound
    ));
    let path = program(dir.path(), "exit 2");
    assert!(matches!(
        probe(&path, Duration::from_secs(1)),
        ExecutableProbe::Installed(ExecutableObservationV1 { version, .. }) if version.is_empty()
    ));
}

#[test]
fn version_probe_keeps_native_startup_writes_in_a_disposable_home() {
    let dir = tempfile::tempdir().unwrap();
    let observed = dir.path().join("observed-home");
    let output_path = observed.to_str().unwrap().replace('\'', "'\\''");
    let path = program(
        dir.path(),
        &format!(
            "test -n \"$HOME\" || exit 4; \
             test -d \"$CODEX_HOME\" || exit 5; \
             test \"$CODEX_HOME\" = \"$HOME/.codex\" || exit 6; \
             test ! -e \"$CODEX_HOME/auth.json\" || exit 7; \
             test ! -e \"$CODEX_HOME/config.toml\" || exit 8; \
             test \"$PWD\" = \"$HOME\" || exit 9; \
             printf '%s' \"$HOME\" > '{output_path}'; \
             touch \"$CODEX_HOME/startup-alias\" || exit 10; \
             printf 'codex-cli 0.162.0-alpha.2\\n'"
        ),
    );
    assert_eq!(
        codex_subscription_client_version(&path).as_deref(),
        Some("0.162.0")
    );
    let private_home = std::fs::read_to_string(observed).unwrap();
    assert!(!private_home.is_empty());
    assert!(
        !Path::new(&private_home).exists(),
        "probe home was not removed"
    );
}

#[test]
fn agent_probe_version_is_diagnostic_and_uses_an_empty_private_home() {
    let dir = tempfile::tempdir().unwrap();
    let path = program(
        dir.path(),
        "test -n \"$HOME\" || exit 4; \
         test \"$CODEX_HOME\" = \"$HOME/.codex\" || exit 5; \
         test ! -e \"$CODEX_HOME/auth.json\" || exit 6; \
         test ! -e \"$CODEX_HOME/config.toml\" || exit 7; \
         printf 'codex-cli 99.123.456-beta.1\\n'",
    );
    let mut completed = 0;
    for _ in 0..64 {
        let found = match probe(&path, Duration::from_secs(1)) {
            ExecutableProbe::Installed(found) => found,
            ExecutableProbe::NotFound => panic!("executable disappeared"),
            ExecutableProbe::Unknown(reason) => {
                panic!("executable probe failed: {reason:?}")
            }
        };
        // A bounded diagnostic may time out before the interpreter runs when probes
        // launch concurrently. A completed probe still proves the child saw an empty home.
        if !found.version.is_empty() {
            assert_eq!(found.version, "99.123.456-beta.1");
            completed += 1;
        }
    }
    assert!(
        completed > 0,
        "no probe verified the private child environment"
    );
}

#[test]
fn output_limit_pipe_closure_preserves_the_probe_failure() {
    let root = tempfile::tempdir().unwrap();
    let path = program(
        root.path(),
        "while :; do printf 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx'; done",
    );
    for _ in 0..16 {
        assert!(matches!(
            bounded_version(&path, Duration::from_secs(10)),
            Err(ProbeFailure::OutputLimit)
        ));
    }
}

#[test]
fn agent_probe_timeout_kills_process_group_without_waiting_for_output_eof() {
    let dir = tempfile::tempdir().unwrap();
    let path = program(dir.path(), "sleep 30 & wait");
    let start = Instant::now();
    assert!(matches!(probe(&path, Duration::from_millis(80)),
        ExecutableProbe::Installed(ExecutableObservationV1 { version, .. }) if version.is_empty()));
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn version_probe_leader_exit_cannot_leave_a_delayed_writer_alive() {
    let root = tempfile::tempdir().unwrap();
    let path = program(
        root.path(),
        r#"(printf 'started' > "$0.child-started"; sleep 0.8; printf 'survived' > "$0.child-survived") &
while test ! -f "$0.child-started"; do sleep 0.01; done
printf 'codex-cli 0.162.0\n'
exit 0"#,
    );
    let mut unrelated = std::process::Command::new("/bin/sleep")
        .arg("10")
        .spawn()
        .unwrap();
    // A lingering orphan zombie may conservatively make cleanup unknown; success is
    // not required, but leaving an active descendant or stopping another group is forbidden.
    let found = probe(&path, Duration::from_secs(2));
    let unrelated_running = unrelated.try_wait().unwrap().is_none();
    let _ = unrelated.kill();
    let _ = unrelated.wait();
    std::thread::sleep(Duration::from_millis(1000));
    assert!(matches!(found, ExecutableProbe::Installed(_)));
    assert!(unrelated_running);
    assert!(root.path().join("agent.child-started").exists());
    assert!(!root.path().join("agent.child-survived").exists());
}

#[test]
fn agent_probe_output_is_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let path = program(
        dir.path(),
        "while :; do printf 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx'; done",
    );
    assert!(matches!(
        probe(&path, Duration::from_secs(1)),
        ExecutableProbe::Installed(ExecutableObservationV1 { version, .. }) if version.is_empty()
    ));
}

#[test]
fn group_writable_executable_and_ancestor_are_allowed() {
    let dir = tempfile::tempdir().unwrap();
    let cask = dir.path().join("Caskroom");
    let bin = dir.path().join("bin");
    std::fs::create_dir(&cask).unwrap();
    std::fs::create_dir(&bin).unwrap();
    let target = program(&cask, "printf 'claude 2.1.231 (Claude Code)\\n'");
    let path = bin.join("claude");
    std::os::unix::fs::symlink(&target, &path).unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o777)).unwrap();
    std::fs::set_permissions(&cask, std::fs::Permissions::from_mode(0o775)).unwrap();

    let ExecutableProbe::Installed(found) = probe(&path, Duration::from_secs(1)) else {
        panic!("group-writable installation was rejected");
    };
    assert_eq!(
        found.canonical_path,
        std::fs::canonicalize(&target).unwrap().to_str().unwrap()
    );
    assert!(found.version.is_empty() || found.version == "2.1.231");
}

#[test]
fn agent_probe_parses_claude_version_without_running_a_process() {
    assert_eq!(
        parse_version(b"claude 2.1.231 (Claude Code)\n"),
        Some("2.1.231".to_owned())
    );
}

#[test]
fn non_executable_file_is_reported_without_launching() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("executed");
    let path = program(dir.path(), &format!("touch '{}'", marker.display()));
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

    assert!(matches!(
        probe(&path, Duration::from_secs(1)),
        ExecutableProbe::Unknown(ProbeFailure::NotExecutable)
    ));
    assert!(!marker.exists());
}

#[test]
#[cfg(target_os = "linux")]
fn agent_probe_busy_executable_stays_bounded_and_succeeds_after_writer_closes() {
    let directory = tempfile::tempdir().unwrap();
    let path = program(directory.path(), "printf 'codex-cli 99.1.2\\n'");
    let writer = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    let started = Instant::now();
    match probe(&path, Duration::from_secs(1)) {
        ExecutableProbe::Unknown(ProbeFailure::LaunchFailed(Some(code))) => {
            assert_eq!(code, nix::errno::Errno::ETXTBSY as i32);
        }
        _ => panic!("a held executable writer must not be reported installed"),
    }
    assert!(started.elapsed() < Duration::from_secs(2));
    drop(writer);
    assert!(matches!(
        probe(&path, Duration::from_secs(1)),
        ExecutableProbe::Installed(_)
    ));
}
