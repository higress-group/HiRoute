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
fn agent_probe_version_is_diagnostic_and_private_environment_is_not_inherited() {
    let dir = tempfile::tempdir().unwrap();
    let path = program(
        dir.path(),
        "test -z \"$HOME\" || exit 4; printf 'codex-cli 99.123.456-beta.1\\n'",
    );
    for _ in 0..64 {
        let found = match probe(&path, Duration::from_secs(1)) {
            ExecutableProbe::Installed(found) => found,
            ExecutableProbe::NotFound => panic!("executable disappeared"),
            ExecutableProbe::Unknown(reason) => {
                panic!("executable probe failed: {reason:?}")
            }
        };
        assert_eq!(found.version, "99.123.456-beta.1");
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
    assert_eq!(found.version, "2.1.231");
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
