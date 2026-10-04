#![cfg(unix)]

use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

const REQUIRED_INSTALLATIONS: [&str; 5] = [
    "HIROUTE_WORKER_CODEX_BINARY",
    "HIROUTE_WORKER_CODEX_ACP_ADAPTER",
    "HIROUTE_WORKER_CLAUDE_BINARY",
    "HIROUTE_WORKER_CLAUDE_ACP_ADAPTER",
    "HIROUTE_WORKER_NODE",
];

pub fn run_real_worker_scenario(mode: &str, expected_scenario: &str) {
    let qoder_script = match mode {
        "HIROUTE_PRODUCT_QODER_MAIN" => Some("qoder_collaboration_product.py"),
        "HIROUTE_PRODUCT_QODER_CORE" => Some("native_context_product.py"),
        "HIROUTE_PRODUCT_QODER_BOUNDARIES" => Some("native_context_boundaries.py"),
        "HIROUTE_PRODUCT_QODER_COMPACTION" => Some("qoder_compaction_product.py"),
        "HIROUTE_PRODUCT_QODER_MODELS" => Some("qoder_model_product.py"),
        _ => None,
    };
    if let Some(script) = qoder_script {
        run_qoder_scenario(script, expected_scenario);
        return;
    }
    let script = match mode {
        "HIROUTE_PRODUCT_NATIVE_CONTEXT" => "native_context_product.py",
        "HIROUTE_PRODUCT_NATIVE_CONTEXT_BOUNDARIES" => "native_context_boundaries.py",
        _ => "delegation_product.py",
    };
    run_scenario(
        script,
        mode,
        expected_scenario,
        &["codex", "claude"],
        &REQUIRED_INSTALLATIONS,
    );
}

fn run_qoder_scenario(script: &str, expected_scenario: &str) {
    let context_variables = if script == "qoder_model_product.py" {
        [
            "HIROUTE_QODER_MODEL_CONTEXT_HOME",
            "HIROUTE_QODER_MODEL_CONFIG_DIR",
        ]
    } else {
        ["HIROUTE_QODER_CONTEXT_HOME", "HIROUTE_QODER_CONFIG_DIR"]
    };
    for name in context_variables {
        let path = PathBuf::from(required(name));
        assert!(
            path.is_absolute() && path.is_dir(),
            "{name} must explicitly name an existing logged-in context"
        );
    }
    run_scenario(
        script,
        "HIROUTE_PRODUCT_QODER",
        expected_scenario,
        &["qoder"],
        &["HIROUTE_WORKER_QODER_BINARY"],
    );
}

fn run_scenario(
    script: &str,
    mode: &str,
    expected_scenario: &str,
    harnesses: &[&str],
    installations: &[&str],
) {
    let workspace = workspace_root();
    let candidate = required("HIROUTE_PRODUCT_CANDIDATE_SHA");
    assert_eq!(candidate.len(), 40, "candidate must be a full Git SHA");
    assert_eq!(
        git_head(&workspace),
        candidate,
        "candidate checkout mismatch"
    );
    for &variable in installations {
        let path = PathBuf::from(required(variable));
        assert!(
            path.is_absolute() && path.is_file(),
            "{variable} must name an installed file"
        );
    }

    build_product_binaries(&workspace);
    for &harness in harnesses {
        let mut command = Command::new(python());
        command
            .arg(workspace.join("crates/daemon/tests/support").join(script))
            .arg(&workspace)
            .arg(&candidate)
            .env("HIROUTE_PRODUCT_WORKER_HARNESS", harness)
            .env(
                "HIROUTE_VALIDATION_PRODUCT_BIN_DIR",
                workspace.join("target/debug"),
            )
            .current_dir(&workspace);
        if script == "native_context_boundaries.py" {
            command.args(["--harness", harness]);
        }
        command.env(mode, "1");
        let output = command
            .output()
            .expect("run the real Worker product scenario");
        assert_scenario(output, expected_scenario, harness, &candidate);
    }
}

fn build_product_binaries(workspace: &Path) {
    let status = Command::new(env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args([
            "build",
            "--locked",
            "-p",
            "hiroute-daemon",
            "--bin",
            "hirouted",
            "-p",
            "hiroute-cli",
            "--bin",
            "hiroute",
        ])
        .current_dir(workspace)
        .status()
        .expect("build production Worker binaries");
    assert!(status.success(), "production Worker binary build failed");
}

fn assert_scenario(output: Output, scenario: &str, harness: &str, candidate: &str) {
    if !output.status.success() {
        panic!(
            "real {harness} Worker scenario failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let report = String::from_utf8(output.stdout).expect("scenario output is UTF-8");
    let report = report
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|value| value.get("scenario").and_then(Value::as_str) == Some(scenario))
        .unwrap_or_else(|| panic!("missing {scenario} report for {harness}: {report}"));
    assert_eq!(report["state"], "green", "scenario report: {report}");
    let expected: &[&str] = match scenario {
        "worker-native-context-core" => &[
            "worker.context.native-skills",
            "worker.context.exact-continue",
        ],
        "worker-native-context-boundaries" => &[
            "worker.context.concurrent-routing",
            "worker.context.cancel-owned-work",
            "worker.context.exact-continue",
        ],
        "qoder-worker-compaction-route" => &[
            "worker.context.tool-summary-route",
            "worker.context.compaction-route",
        ],
        "qoder-main-agent-delegation" => &[
            "agent.collaboration.user-skill",
            "agent.collaboration.public-worker-delegation",
            "agent.collaboration.disable-owned-skill",
        ],
        "qoder-persisted-model-routes" => &[
            "agent.models.persisted-routes",
            "agent.models.credential-rotation",
            "agent.models.independent-restore",
            "agent.models.default-reference-guard",
        ],
        _ => &[],
    };
    if !expected.is_empty() {
        let cases = report["cases"].as_array().expect("required case results");
        assert_eq!(cases.len(), expected.len(), "scenario report: {report}");
        for &id in expected {
            assert_eq!(
                cases
                    .iter()
                    .filter(|case| case["id"] == id && case["state"] == "green")
                    .count(),
                1,
                "missing, duplicate or non-green case {id}: {report}"
            );
        }
    }
    assert_eq!(report["candidate"], candidate, "scenario report: {report}");
    assert_eq!(
        report["worker_harness"], harness,
        "scenario report: {report}"
    );
    println!("{report}");
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonical workspace root")
}

fn git_head(workspace: &Path) -> String {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(workspace)
        .output()
        .expect("read exact candidate SHA");
    assert!(output.status.success(), "git rev-parse failed");
    String::from_utf8(output.stdout)
        .expect("Git SHA is UTF-8")
        .trim()
        .to_owned()
}

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("{name} is required for this opt-in product test"))
}

fn python() -> String {
    env::var("PYTHON").unwrap_or_else(|_| "python3".into())
}
