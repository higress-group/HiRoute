use std::path::Path;
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};

fn isolated_cli(runtime: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_hiroute"));
    command
        .env("HOME", runtime)
        .env("HIROUTE_RUNTIME_DIR", runtime)
        .args(["--output", "json"]);
    command
}

#[test]
fn released_control_command_reaches_isolated_transport() {
    let runtime = tempfile::tempdir().unwrap();
    let output = isolated_cli(runtime.path())
        .args(["system", "status"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(6));
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
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

#[test]
fn released_allowance_leaves_accept_discovered_examples_and_validate_before_transport() {
    let runtime = tempfile::tempdir().unwrap();
    let request_file = runtime.path().join("key-budget-request.json");
    for (leaf, request_schema, example_source) in [
        (
            "get",
            "hiroute.compute-key-budgets-query/v1",
            "--request-stdin",
        ),
        (
            "preview",
            "hiroute.compute-key-budget-save-request/v1",
            "--request-file",
        ),
    ] {
        let command_id = format!("compute.budgets.{leaf}");
        let output = isolated_cli(runtime.path())
            .args([
                "schema",
                "show",
                "--command-id",
                &command_id,
                "--include-schema",
            ])
            .output()
            .unwrap();
        assert!(output.status.success(), "{command_id}");
        let discovery: Value = serde_json::from_slice(&output.stdout).unwrap();
        let descriptor = &discovery["data"]["descriptor"];
        assert_eq!(descriptor["request_schema"], request_schema);
        assert_eq!(
            descriptor["stdin_channels"],
            json!(["request_stdin", "request_file"])
        );
        let usage = format!(
            "hiroute compute budgets {leaf} (--request-stdin | --request-file <PATH>) --output json"
        );
        assert_eq!(descriptor["help"]["usage"], usage);
        let help = isolated_cli(runtime.path())
            .args(["compute", "budgets", leaf, "--help"])
            .output()
            .unwrap();
        assert!(help.status.success(), "{command_id}");
        let help = String::from_utf8(help.stdout).unwrap();
        assert!(help.lines().any(|line| line.trim() == usage));
        assert!(help.contains(&format!("hiroute compute budgets {leaf} {example_source}")));

        let mut request = discovery["data"]["schemas"]["examples"][0].clone();
        assert_cli_error(
            budget_request_on_stdin(runtime.path(), leaf, &request),
            6,
            "DAEMON_UNAVAILABLE",
        );
        std::fs::write(&request_file, serde_json::to_vec(&request).unwrap()).unwrap();
        let output = isolated_cli(runtime.path())
            .args(["compute", "budgets", leaf, "--request-file"])
            .arg(&request_file)
            .output()
            .unwrap();
        assert_cli_error(output, 6, "DAEMON_UNAVAILABLE");

        if leaf == "get" {
            request["source_id"] = json!(42);
        } else {
            *request
                .pointer_mut("/key_budget_edits/0/assignment/policy/amount_micros")
                .unwrap() = json!(-1);
        }
        assert_cli_error(
            budget_request_on_stdin(runtime.path(), leaf, &request),
            2,
            "INVALID_ARGUMENTS",
        );
    }
}

fn budget_request_on_stdin(runtime: &Path, leaf: &str, request: &Value) -> Output {
    let mut child = isolated_cli(runtime)
        .args(["compute", "budgets", leaf, "--request-stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    serde_json::to_writer(child.stdin.take().unwrap(), request).unwrap();
    child.wait_with_output().unwrap()
}

fn assert_cli_error(output: Output, status: i32, code: &str) {
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output.status.code(), Some(status), "{value}");
    assert_eq!(value["error"]["code"], code, "{value}");
}
