//! Unit runtime fixtures cannot inherit a native installation from the test host.
use super::*;

#[cfg(unix)]
#[test]
fn runtime_fixture_does_not_execute_ambient_codex() {
    use std::os::unix::fs::PermissionsExt;
    const CHILD: &str = "HIROUTE_RUNTIME_FIXTURE_CODEX";
    const CASE: &str = "runtime::tests::fixture_isolation::runtime_fixture_does_not_execute_ambient_codex";
    if let Some(executable) = std::env::var_os(CHILD) {
        let root = tempfile::tempdir().unwrap();
        let runtime = fixture_runtime(
            &root,
            Arc::new(FakeBackend::default()),
            Arc::new(FakeControl::default()),
            2,
        );
        let outcome = runtime.start();
        assert!(
            !PathBuf::from(executable).with_file_name("codex.probed").exists(),
            "the runtime fixture executed the host installation: {outcome:?}"
        );
        assert!(matches!(outcome, Ok(CpaHealth::Ready { .. })));
        runtime.shutdown().unwrap();
        return;
    }

    let installation = tempfile::tempdir().unwrap();
    let executable = installation.path().join("codex");
    std::fs::write(
        &executable,
        "#!/bin/sh\nprintf x > \"$0.probed\"\nsleep 1\nprintf 'codex-cli 0.162.0\\n'\n",
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    // Only the child test receives this PATH; the current process and user's
    // native homes/credentials are untouched.
    let path = std::env::join_paths([
        installation.path(),
        Path::new("/usr/bin"),
        Path::new("/bin"),
    ])
    .unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", CASE, "--test-threads=1"])
        .env(CHILD, &executable)
        .env("PATH", path)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success()
            && stdout.lines().any(|line| line == format!("test {CASE} ... ok")),
        "isolated fixture regression must execute its exact case: {stdout} {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
