//! Native npm bins keep argv and engine identity without a Node dependency.
use super::*;

#[test]
fn native_npm_pe_runs_without_node_and_invalidates_changed_engine_identity() {
    use hiroute_platform::windows::security;
    let root = super::super::probe_files::temporary("hiroute-native-npm-").unwrap();
    let prefix = root.path().join("npm 用户");
    let package = prefix.join("node_modules/@anthropic-ai/claude-code");
    let bin = package.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let shim = prefix.join("claude.cmd");
    std::fs::write(
        &shim,
        b"@echo this shim must never execute\r\nexit /b 97\r\n",
    )
    .unwrap();
    std::fs::write(
        package.join("package.json"),
        br#"{"name":"@anthropic-ai/claude-code","bin":{"claude":"bin/claude.EXE"}}"#,
    )
    .unwrap();
    let entry = bin.join("claude.EXE");
    std::fs::copy(std::env::current_exe().unwrap(), &entry).unwrap();
    let args = vec![PathBuf::from(""), PathBuf::from("中文 ' & % !")];
    let (program, observed) = windows_agent_command(&shim, &args, "").unwrap();
    assert_eq!(observed, args);
    assert_eq!(
        security::file_identity(&security::open_identity(&program, false).unwrap()).unwrap(),
        security::file_identity(&security::open_identity(&entry, false).unwrap()).unwrap()
    );
    let before = super::super::native_ingress_probe::binary_identity(&shim).unwrap();
    let mut command = Command::new(&shim);
    command
        .args([
            "--ignored",
            "--exact",
            super::tests::ENV_FIXTURE,
            "--nocapture",
        ])
        .env_clear()
        .env("HOME", root.path())
        .env("PATH", "")
        .current_dir(root.path());
    let output = bounded(&command, false, Duration::from_secs(10), 4096).unwrap();
    assert!(String::from_utf8_lossy(&output).contains("WINDOW_NATIVE_SYSTEM_ENV="));
    std::fs::OpenOptions::new()
        .write(true)
        .open(&entry)
        .unwrap()
        .set_len(1)
        .unwrap();
    assert_ne!(
        super::super::native_ingress_probe::binary_identity(&shim).unwrap(),
        before
    );
}
