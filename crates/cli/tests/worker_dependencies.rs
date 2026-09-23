#![cfg(unix)]

use hiroute_application::ApplicationService;
use hiroute_daemon::control::{ProductionControlRuntime, start_control};
use serde_json::{Value, json};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

const CHILD: &str = "HIROUTE_WORKER_DEPENDENCIES_TEST_CHILD";

fn write_dependency(path: &Path, executable: bool) {
    std::fs::write(path, b"fixture").unwrap();
    let mode = if executable { 0o700 } else { 0o600 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

fn run_cli(runtime_root: &Path, arguments: &[&str], stdin: Option<&[u8]>) -> Value {
    let mut command = Command::new(env!("CARGO_BIN_EXE_hiroute"));
    command
        .args(arguments)
        .env("HIROUTE_RUNTIME_DIR", runtime_root)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    if let Some(stdin) = stdin {
        child.stdin.take().unwrap().write_all(stdin).unwrap();
    }
    let output = child.wait_with_output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn candidate_path(discovery: &Value, component: &str) -> String {
    discovery["data"]["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| {
            candidate["harness"] == "codex_cli"
                && candidate["component"] == component
                && candidate["state"] == "found"
        })
        .and_then(|candidate| candidate["path"].as_str())
        .unwrap_or_else(|| panic!("missing found {component} candidate: {discovery}"))
        .to_owned()
}

#[test]
fn public_cli_discover_select_and_same_request_replay_are_self_contained() {
    if std::env::var_os(CHILD).is_none() {
        let home = tempfile::tempdir().unwrap();
        std::fs::set_permissions(home.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let bin = home.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        write_dependency(&bin.join("codex"), true);
        write_dependency(&bin.join("codex-acp"), false);
        write_dependency(&bin.join("node"), true);
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "public_cli_discover_select_and_same_request_replay_are_self_contained",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("HOME", home.path())
            .env("PATH", &bin)
            .env_remove("CODEX_HOME")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("NPM_CONFIG_PREFIX")
            .env_remove("npm_config_prefix")
            .env_remove("NPM_CONFIG_CACHE")
            .env_remove("npm_config_cache")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }

    let root = tempfile::tempdir().unwrap();
    let runtime = ProductionControlRuntime::open(root.path().join("storage")).unwrap();
    let runtime_root = root.path().join("runtime");
    let mut control = start_control(
        ApplicationService::new(runtime.application_ports()),
        &runtime_root,
    )
    .unwrap();

    let discovery = run_cli(
        &runtime_root,
        &[
            "worker",
            "dependencies",
            "discover",
            "--harness",
            "codex_cli",
            "--output",
            "json",
        ],
        None,
    );
    assert_eq!(discovery["status"], "succeeded");
    assert_eq!(
        discovery["data"]["selection_revisions"],
        json!([{"harness": "codex_cli", "revision": 0}])
    );
    let request = json!({
        "harness": "codex_cli",
        "adapter_path": candidate_path(&discovery, "adapter"),
        "cli_path": candidate_path(&discovery, "cli"),
        "node_path": candidate_path(&discovery, "node"),
        "expected_selection_revision": 0,
    });
    let request = serde_json::to_vec(&request).unwrap();
    let select_arguments = [
        "worker",
        "dependencies",
        "select",
        "--request-stdin",
        "--output",
        "json",
    ];
    let first = run_cli(&runtime_root, &select_arguments, Some(&request));
    assert_eq!(first["status"], "accepted");
    assert_eq!(
        first["data"]["selection_revisions"],
        json!([{"harness": "codex_cli", "revision": 1}])
    );

    // This is the recovery action available to an installed Agent after an uncertain response:
    // replay the preserved request body. It must return the original Operation and not write a
    // second selection revision.
    let replay = run_cli(&runtime_root, &select_arguments, Some(&request));
    assert_eq!(replay["status"], "accepted");
    assert_eq!(replay["operation"], first["operation"]);
    assert_eq!(
        replay["data"]["selection_revisions"],
        json!([{"harness": "codex_cli", "revision": 1}])
    );

    let observed = run_cli(
        &runtime_root,
        &[
            "worker",
            "dependencies",
            "discover",
            "--harness",
            "codex_cli",
            "--output",
            "json",
        ],
        None,
    );
    assert_eq!(observed["data"]["selection_revisions"][0]["revision"], 1);
    assert_eq!(observed["data"]["selected"], replay["data"]["selected"]);

    control.shutdown();
    control.join(Duration::from_secs(10)).unwrap();
}
