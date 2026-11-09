use std::io::Write;
use std::process::{Command, Stdio};

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

/// The released allowance leaves must encode their strict request and reach Local Control.
/// Before the codecs existed they failed argument resolution with UNKNOWN_COMMAND, which hid a
/// real consumer behind a usage error even for the exact request shape the schema advertises.
#[test]
fn released_allowance_leaves_encode_requests_and_reach_transport() {
    let runtime = tempfile::tempdir().unwrap();
    let cases: [(&[&str], &[u8]); 2] = [
        (
            &["compute", "budgets", "get", "--request-stdin"],
            br#"{"source_id":null,"cursor":null}"#,
        ),
        (
            &["compute", "budgets", "preview", "--request-stdin"],
            br#"{"change":{"schema":"hiroute.compute-management-change/v3","api_link":null,"default_preferences":{},"change":{"schema":"hiroute.compute-management-change/v2","subject":{"kind":"saved_source","source_id":"source/fixture"},"expected_revisions":{"target":0},"selected_model_refs":[],"intent":"save_ready","key_edits":[]}},"network_edit":null,"key_budget_edits":[]}"#,
        ),
    ];
    for (arguments, request) in cases {
        let mut child = Command::new(env!("CARGO_BIN_EXE_hiroute"))
            .env("HOME", runtime.path())
            .env("HIROUTE_RUNTIME_DIR", runtime.path())
            .args(arguments)
            .args(["--output", "json"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(request).unwrap();
        let output = child.wait_with_output().unwrap();
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_ne!(
            value["error"]["code"],
            "UNKNOWN_COMMAND",
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert_ne!(
            value["error"]["code"],
            "INVALID_ARGUMENTS",
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(output.status.code(), Some(6), "{arguments:?}");
        assert_eq!(value["error"]["code"], "DAEMON_UNAVAILABLE");
    }
}
