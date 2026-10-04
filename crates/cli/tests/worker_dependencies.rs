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
    serde_json::from_str(&run_cli_output(runtime_root, arguments, stdin)).unwrap()
}

fn run_cli_output(runtime_root: &Path, arguments: &[&str], stdin: Option<&[u8]>) -> String {
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
    String::from_utf8(output.stdout).unwrap()
}

fn candidate_path(discovery: &Value, harness: &str, component: &str) -> String {
    discovery["data"]["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| {
            candidate["harness"] == harness
                && candidate["component"] == component
                && candidate["state"] == "found"
        })
        .and_then(|candidate| candidate["path"].as_str())
        .unwrap_or_else(|| panic!("missing found {harness} {component} candidate: {discovery}"))
        .to_owned()
}

fn isolated_dependency_environment(test_name: &str, dependencies: &[(&str, bool)]) -> bool {
    if std::env::var(CHILD).as_deref() == Ok(test_name) {
        return true;
    }
    let home = tempfile::tempdir().unwrap();
    std::fs::set_permissions(home.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let bin = home.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    for (name, executable) in dependencies {
        write_dependency(&bin.join(name), *executable);
    }
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test_name, "--nocapture"])
        .env(CHILD, test_name)
        .env("HOME", home.path())
        .env("PATH", &bin)
        .env_remove("CODEX_HOME")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("QODER_CONFIG_DIR")
        .env_remove("NPM_CONFIG_PREFIX")
        .env_remove("npm_config_prefix")
        .env_remove("NPM_CONFIG_CACHE")
        .env_remove("npm_config_cache")
        .status()
        .unwrap();
    assert!(status.success());
    false
}

#[test]
fn public_cli_discover_select_and_same_request_replay_are_self_contained() {
    if !isolated_dependency_environment(
        "public_cli_discover_select_and_same_request_replay_are_self_contained",
        &[("codex", true), ("codex-acp", false), ("node", true)],
    ) {
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
        "adapter_path": candidate_path(&discovery, "codex_cli", "adapter"),
        "cli_path": candidate_path(&discovery, "codex_cli", "cli"),
        "node_path": candidate_path(&discovery, "codex_cli", "node"),
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

#[test]
fn public_cli_qoder_native_selection_replays_without_adapter_or_node() {
    if !isolated_dependency_environment(
        "public_cli_qoder_native_selection_replays_without_adapter_or_node",
        &[("qodercli", true)],
    ) {
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
    let discover_arguments = [
        "worker",
        "dependencies",
        "discover",
        "--harness",
        "qoder_cli",
        "--output",
        "json",
    ];
    let discovery = run_cli(&runtime_root, &discover_arguments, None);
    assert_eq!(discovery["status"], "succeeded");
    assert_eq!(
        discovery["data"]["selection_revisions"],
        json!([{"harness": "qoder_cli", "revision": 0}])
    );
    assert!(discovery["data"]["selected"].as_array().unwrap().is_empty());
    assert!(
        discovery["data"]["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .all(|candidate| candidate["harness"] == "qoder_cli"
                && candidate["component"] == "cli")
    );
    let cli_path = candidate_path(&discovery, "qoder_cli", "cli");
    let request = serde_json::to_vec(&json!({
        "harness": "qoder_cli",
        "cli_path": cli_path,
        "expected_selection_revision": 0,
    }))
    .unwrap();
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
    let selected = json!([{"harness": "qoder_cli", "cli_path": cli_path}]);
    assert_eq!(first["data"]["selected"], selected);
    assert_eq!(
        first["data"]["selection_revisions"],
        json!([{"harness": "qoder_cli", "revision": 1}])
    );

    let replay = run_cli(&runtime_root, &select_arguments, Some(&request));
    assert_eq!(replay["status"], "accepted");
    assert_eq!(replay["operation"], first["operation"]);
    assert_eq!(replay["data"]["selected"], selected);
    assert_eq!(
        replay["data"]["selection_revisions"],
        first["data"]["selection_revisions"]
    );

    let observed = run_cli(&runtime_root, &discover_arguments, None);
    assert_eq!(observed["data"]["selected"], selected);
    assert_eq!(observed["data"]["selection_revisions"][0]["revision"], 1);

    let mut text_arguments = discover_arguments;
    text_arguments[6] = "text";
    let text = run_cli_output(&runtime_root, &text_arguments, None);
    assert_eq!(
        text.lines()
            .filter(|line| line.starts_with("selected\t"))
            .collect::<Vec<_>>(),
        [format!("selected\tQoderCli\t{cli_path}")]
    );
    text_arguments[6] = "quiet";
    assert_eq!(
        run_cli_output(&runtime_root, &text_arguments, None),
        "qoder_cli\n"
    );

    control.shutdown();
    control.join(Duration::from_secs(10)).unwrap();
}
