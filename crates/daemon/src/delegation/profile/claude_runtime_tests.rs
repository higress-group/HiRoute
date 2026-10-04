use super::*;

#[test]
fn only_known_claude_release_format_and_supported_major_can_borrow_provider_settings() {
    for version in [
        "2.1.231 (Claude Code)\n",
        "claude 2.1.232 (Claude Code)\n",
        "2.2.0 (Claude Code)",
    ] {
        assert!(
            supports_host_managed_provider(version.as_bytes()),
            "{version}"
        );
    }
    for version in [
        "2.1.230 (Claude Code)",
        "2.0.999 (Claude Code)",
        "1.9.999 (Claude Code)",
        "3.0.0 (Claude Code)",
        "2.1.231",
        "unknown 2.1.231 (Claude Code)",
        "2.1.231-beta (Claude Code)",
        "2.1.231+local (Claude Code)",
        "2.01.231 (Claude Code)",
        "2.1.231.1 (Claude Code)",
        "2.1.999999999999999999999 (Claude Code)",
        "2.1.231 (Claude Code)\nwarning: partial startup",
        "",
    ] {
        assert!(
            !supports_host_managed_provider(version.as_bytes()),
            "{version}"
        );
    }
    assert!(!supports_host_managed_provider(&[255]));
}

#[cfg(unix)]
mod process {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::process::Command;
    use std::time::Instant;

    fn executable(root: &Path, body: &str) -> PathBuf {
        let path = root.join("selected-claude");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    #[test]
    fn selected_cli_runs_only_version_without_inherited_context_or_secrets() {
        let root = tempfile::tempdir().unwrap();
        let binary = executable(
            root.path(),
            r#"test "$#" -eq 1 && test "$1" = '--version' || exit 2
test -z "$HOME$CODEX_HOME$CLAUDE_CONFIG_DIR$ANTHROPIC_AUTH_TOKEN$HIROUTE_RUN_TOKEN" || exit 3
test "$PATH" = '/selected/node:/usr/bin:/bin' || exit 4
printf '2.1.231 (Claude Code)\n'"#,
        );
        assert_eq!(
            require_host_managed_provider(&binary, "/selected/node:/usr/bin:/bin"),
            Ok(())
        );
    }

    #[test]
    fn old_failed_missing_or_oversized_cli_metadata_cannot_authorize_borrowing() {
        for body in [
            "printf '2.1.230 (Claude Code)\\n'",
            "printf '2.1.231 (Claude Code)\\n'; exit 7",
            "printf '2.1.231 (Claude Code)\\n' >&2",
            "while :; do printf 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx'; done",
        ] {
            let root = tempfile::tempdir().unwrap();
            let binary = executable(root.path(), body);
            assert_eq!(
                require_host_managed_provider(&binary, "/usr/bin:/bin"),
                Err(DelegationErrorV1::CapabilityUnavailable)
            );
        }
        let root = tempfile::tempdir().unwrap();
        assert_eq!(
            require_host_managed_provider(&root.path().join("missing"), "/usr/bin:/bin"),
            Err(DelegationErrorV1::CapabilityUnavailable)
        );
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn transient_executable_writer_retries_metadata_without_executing_it_twice() {
        use std::process::Stdio;

        let root = tempfile::tempdir().unwrap();
        let binary = executable(
            root.path(),
            r#"printf 'launched\n' >> "$0.started"
printf '2.1.231 (Claude Code)\n'"#,
        );
        let mut writer = Some(
            std::fs::OpenOptions::new()
                .write(true)
                .open(&binary)
                .unwrap(),
        );
        let mut command = Command::new(&binary);
        command.arg("--version").stdout(Stdio::piped());
        let mut attempts = 0;
        let mut first_error = None;
        let child = spawn_version_command(
            || {
                attempts += 1;
                let result = command.spawn();
                if attempts == 1 {
                    first_error = result.as_ref().err().and_then(|error| error.raw_os_error());
                    // Release after a real kernel ETXTBSY, with no timer/thread race.
                    drop(writer.take());
                }
                result
            },
            Instant::now(),
            Duration::from_secs(1),
        )
        .unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(first_error, Some(nix::errno::Errno::ETXTBSY as i32));
        assert!(attempts >= 2);
        assert!(output.status.success());
        assert!(supports_host_managed_provider(&output.stdout));
        assert_eq!(
            std::fs::read_to_string(root.path().join("selected-claude.started")).unwrap(),
            "launched\n"
        );
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn persistent_executable_writer_stays_bounded_and_never_runs_metadata() {
        let root = tempfile::tempdir().unwrap();
        let binary = executable(
            root.path(),
            r#"printf 'launched\n' > "$0.started"
printf '2.1.231 (Claude Code)\n'"#,
        );
        let writer = std::fs::OpenOptions::new()
            .write(true)
            .open(&binary)
            .unwrap();
        let started = Instant::now();
        assert_eq!(
            require_version(&binary, "/usr/bin:/bin", Duration::from_secs(1)),
            Err(DelegationErrorV1::CapabilityUnavailable)
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(!root.path().join("selected-claude.started").exists());
        drop(writer);
    }

    #[test]
    fn timeout_stops_owned_descendants_without_stopping_unrelated_processes() {
        let root = tempfile::tempdir().unwrap();
        let binary = executable(
            root.path(),
            r#"(printf 'started' > "$0.child-started"; sleep 0.8; printf 'survived' > "$0.child-survived") &
wait"#,
        );
        let mut unrelated = Command::new("/bin/sleep").arg("10").spawn().unwrap();
        let started = Instant::now();
        let result = require_version(&binary, "/usr/bin:/bin", Duration::from_millis(300));
        let elapsed = started.elapsed();
        let unrelated_running = unrelated.try_wait().unwrap().is_none();
        let _ = unrelated.kill();
        let _ = unrelated.wait();
        std::thread::sleep(Duration::from_millis(1000));
        assert_eq!(result, Err(DelegationErrorV1::CapabilityUnavailable));
        assert!(elapsed < Duration::from_secs(2));
        assert!(unrelated_running);
        assert!(root.path().join("selected-claude.child-started").exists());
        assert!(!root.path().join("selected-claude.child-survived").exists());
    }

    #[test]
    fn exit_observation_keeps_the_child_owned_until_group_cleanup() {
        use std::os::unix::process::CommandExt;

        let root = tempfile::tempdir().unwrap();
        let binary = executable(root.path(), "exit 0");
        let mut child = VersionChild {
            child: Command::new(binary).process_group(0).spawn().unwrap(),
            signals_closed: false,
        };
        let started = Instant::now();
        while observe_version_child(&child).unwrap().is_none() {
            assert!(started.elapsed() < Duration::from_secs(2));
            std::thread::sleep(Duration::from_millis(5));
        }
        // A destructive wait would lose child ownership here. Repeat the OS observation
        // before cleanup to prove the exited leader still reserves its PID and PGID.
        assert_eq!(observe_version_child(&child), Ok(Some(true)));
        assert_eq!(stop_version_child(&mut child), Ok(()));
        assert!(child.child.try_wait().unwrap().unwrap().success());
        // The already-reaped handle must not authorize any subsequent group signal.
        assert_eq!(
            stop_version_child(&mut child),
            Err(DelegationErrorV1::CapabilityUnavailable)
        );
    }

    #[test]
    fn successful_leader_exit_still_stops_descendants_holding_stdout() {
        let root = tempfile::tempdir().unwrap();
        let binary = executable(
            root.path(),
            r#"(printf 'started' > "$0.child-started"; sleep 0.8; printf 'survived' > "$0.child-survived") &
while test ! -f "$0.child-started"; do sleep 0.01; done
printf '2.1.231 (Claude Code)\n'
exit 0"#,
        );
        let started = Instant::now();
        let result = require_version(&binary, "/usr/bin:/bin", Duration::from_secs(2));
        let elapsed = started.elapsed();
        std::thread::sleep(Duration::from_millis(1000));
        assert_eq!(result, Ok(()));
        assert!(elapsed < Duration::from_secs(2));
        assert!(root.path().join("selected-claude.child-started").exists());
        assert!(!root.path().join("selected-claude.child-survived").exists());
    }
}
