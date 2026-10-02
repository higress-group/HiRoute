#![cfg(unix)]
use serde_json::{Value, json};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::process::{Command, Stdio};

fn cli(root: &std::path::Path, payload: Value, secret: Option<&[u8]>) -> (i32, Value) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_hiroute"));
    cmd.args(["work-plans", "list", "--request-stdin", "--output", "json"])
        .env("HIROUTE_RUNTIME_DIR", root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let pipe = secret.map(|bytes| {
        let (read, write) = nix::unistd::pipe().unwrap();
        nix::unistd::write(&write, bytes).unwrap();
        drop(write);
        cmd.args(["--capability-fd", &read.as_raw_fd().to_string()]);
        read
    });
    let mut child = cmd.spawn().unwrap();
    drop(pipe);
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.to_string().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    if let Some(secret) = secret {
        assert!(!output.stdout.windows(secret.len()).any(|w| w == secret));
    }
    (
        output.status.code().unwrap(),
        serde_json::from_slice(&output.stdout).unwrap(),
    )
}

#[test]
fn work_plans_planned_command_is_not_public_cli() {
    let root = tempfile::tempdir().unwrap();
    let secret = b"fixture-unexposed-collaboration-credential";
    let (exit, blocked) = cli(
        root.path(),
        json!({"workspace_id":"personal/default", "context_id":"owner", "grant_id":"collaboration-grant/directory"}),
        Some(secret),
    );
    assert_eq!(exit, 2, "{blocked}");
    assert_eq!(blocked["error"]["code"], "UNKNOWN_COMMAND");
}
