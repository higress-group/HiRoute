use std::process::Command;

#[test]
fn released_control_command_reaches_isolated_transport() {
    let runtime = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_hiroute"))
        .env("HOME", runtime.path())
        .env("HIROUTE_RUNTIME_DIR", runtime.path())
        .args(["system", "status", "--output", "json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(6));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["error"]["code"], "DAEMON_UNAVAILABLE");
    let stderr = String::from_utf8(output.stderr).unwrap();
    for hint in [
        "hiroute service status",
        "hiroute service start",
        "Desktop",
        "隔离实例",
        "不会自动启动或重放请求",
    ] {
        assert!(stderr.contains(hint));
    }
}
