#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use hiroute_application_api::{
    ClientHelloV1, LOCAL_CONTROL_SCHEMA_V2, LocalControlWireRequestV2, MACHINE_ENVELOPE_SCHEMA_V2,
    MachineEnvelopeV2, MachineStatus, ServerHelloV1,
};
use serde::Deserialize;
use serde_json::{Value, json};

const SCENARIO: &str =
    include_str!("../../../e2e/product/scenarios/control/pre-gateway-compute-routing.v1.json");
const GOLDEN: &str =
    include_str!("../../../e2e/product/golden/control/pre-gateway-compute-routing.v1.json");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScenarioV1 {
    schema: String,
    process: String,
    scenario_id: String,
    command_lifecycle: String,
    released_commands: Vec<String>,
    compute_focused_state: String,
    routing_preview_focused_state: String,
    routing_apply_focused_state: String,
    routing_apply_error: String,
    production_subprocess_state: String,
    production_catalog_source: String,
    storage_catalog_tampering: String,
    daemon_stop: String,
    test_process_expected_exit: i32,
    cli_process_expected_exit: i32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BoundaryV1 {
    schema: String,
    process: String,
    production_catalog_trust: String,
    fixture_trust_is_production_fallback: bool,
    cli_direct_business_dependencies: Vec<String>,
    local_control_operations: Vec<String>,
    application_entrypoints: u8,
    preview_database_writes: u8,
    compute_apply_operation_journaled: bool,
    routing_apply_operation_writes: u8,
    routing_apply_plan_writes: u8,
    routing_apply_publication_writes: u8,
    routing_apply_capability_consumed: bool,
    final_gateway_installer: String,
    release_artifact_generation_owner: String,
    public_command_promotion_owner: String,
}

struct ProductBinaries {
    hiroute: PathBuf,
    hirouted: PathBuf,
}

struct OwnedChild(Child);

impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn production_subprocess_uses_embedded_catalog_and_ignores_storage_tampering() {
    let scenario: ScenarioV1 = serde_json::from_str(SCENARIO).unwrap();
    let boundary: BoundaryV1 = serde_json::from_str(GOLDEN).unwrap();
    assert_contract(&scenario, &boundary);

    let workspace = workspace_root();
    let binaries = build_product_binaries(&workspace);
    let temporary = tempfile::tempdir().unwrap();
    let storage_root = temporary.path().join("storage");
    let runtime_root = temporary.path().join("runtime");
    let home = temporary.path().join("home");
    std::fs::create_dir(&home).unwrap();
    std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
    install_nonmatching_catalog(&storage_root);

    let mut daemon = OwnedChild(
        Command::new(&binaries.hirouted)
            .args([
                "--role",
                "control",
                "--storage-root",
                storage_root.to_str().unwrap(),
                "--runtime-root",
                runtime_root.to_str().unwrap(),
            ])
            .env("HOME", &home)
            .env_remove("CODEX_HOME")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("ANTHROPIC_AUTH_TOKEN")
            .env_remove("ANTHROPIC_BASE_URL")
            .env_remove("ANTHROPIC_MODEL")
            .env_remove("ANTHROPIC_DEFAULT_OPUS_MODEL")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("launch production hirouted"),
    );
    let endpoint = runtime_root.join("hiroute/control.sock");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !endpoint.exists() {
        assert!(
            daemon.0.try_wait().unwrap().is_none(),
            "hirouted exited before ready"
        );
        assert!(Instant::now() < deadline, "hirouted readiness timed out");
        std::thread::sleep(Duration::from_millis(20));
    }

    let compute = observe(
        Command::new(&binaries.hiroute)
            .args(["compute", "scan", "--output", "json", "--non-interactive"])
            .env("HIROUTE_RUNTIME_DIR", &runtime_root)
            .output()
            .expect("launch production hiroute compute scan"),
    );
    assert_eq!(compute.process_exit, scenario.cli_process_expected_exit);
    assert_eq!(compute.envelope.status, MachineStatus::Succeeded);
    assert!(compute.envelope.error.is_none());
    let internal = control_call(&endpoint, "ScanCompute", json!({}));
    assert_eq!(internal.status, MachineStatus::Succeeded);
    assert!(internal.error.is_none());
    assert_eq!(compute.envelope.data, internal.data);
    assert_eq!(
        std::fs::read(storage_root.join("release-facts/current/manifest.json")).unwrap(),
        b"{\"schema\":\"untrusted\"}\n"
    );
    assert!(daemon.0.try_wait().unwrap().is_none());

    let help = Command::new(&binaries.hiroute)
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.contains("compute"));
    assert!(help.contains("routing"));

    let schema = observe(
        Command::new(&binaries.hiroute)
            .args(["schema", "list", "--output", "json", "--non-interactive"])
            .output()
            .unwrap(),
    );
    assert_eq!(schema.process_exit, 0);
    let schema_data = schema.envelope.data.unwrap();
    let commands = schema_data["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|command| command["command_id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(
        scenario
            .released_commands
            .iter()
            .all(|released| commands.contains(&released.as_str()))
    );
}

fn control_call(endpoint: &Path, operation_id: &str, payload: Value) -> MachineEnvelopeV2<Value> {
    let mut stream = UnixStream::connect(endpoint).expect("internal Local Control connection");
    serde_json::to_writer(
        &mut stream,
        &ClientHelloV1 {
            api_version: LOCAL_CONTROL_SCHEMA_V2,
            machine_schema_version: MACHINE_ENVELOPE_SCHEMA_V2,
            client_name: "pre-gateway-product-test".into(),
            client_version: env!("CARGO_PKG_VERSION").into(),
        },
    )
    .unwrap();
    stream.write_all(b"\n").unwrap();
    stream.flush().unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    serde_json::from_str::<ServerHelloV1>(&line).unwrap();
    serde_json::to_writer(
        &mut stream,
        &LocalControlWireRequestV2 {
            schema_version: LOCAL_CONTROL_SCHEMA_V2,
            request_id: "pre-gateway-internal-compute".into(),
            operation_id: operation_id.into(),
            payload,
            protected_grant: None,
        },
    )
    .unwrap();
    stream.write_all(b"\n").unwrap();
    stream.flush().unwrap();
    line.clear();
    reader.read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

struct CliObservation {
    process_exit: i32,
    envelope: MachineEnvelopeV2<Value>,
}

fn observe(output: Output) -> CliObservation {
    assert_eq!(
        output
            .stdout
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .count(),
        1
    );
    assert!(output.stderr.is_empty());
    CliObservation {
        process_exit: output.status.code().expect("hiroute was not signalled"),
        envelope: serde_json::from_slice(&output.stdout).expect("one strict machine envelope"),
    }
}

fn install_nonmatching_catalog(storage_root: &Path) {
    hiroute_product_e2e::smoke::fixtures::install_storage_catalog_tampering(storage_root).unwrap();
}

fn build_product_binaries(workspace: &Path) -> ProductBinaries {
    if std::env::var_os("HIROUTE_VALIDATION_EXECUTION").is_some() {
        let directory = PathBuf::from(
            std::env::var_os("HIROUTE_VALIDATION_PRODUCT_BIN_DIR")
                .expect("prepared Control binaries missing"),
        );
        let binaries = ProductBinaries {
            hiroute: directory.join("hiroute"),
            hirouted: directory.join("hirouted"),
        };
        assert!(binaries.hiroute.is_file() && binaries.hirouted.is_file());
        return binaries;
    }
    let status = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .current_dir(workspace)
        .args([
            "build",
            "-p",
            "hiroute-cli",
            "--bin",
            "hiroute",
            "-p",
            "hiroute-daemon",
            "--bin",
            "hirouted",
        ])
        .status()
        .expect("build production binaries");
    assert!(status.success());
    let suffix = std::env::consts::EXE_SUFFIX;
    ProductBinaries {
        hiroute: workspace.join(format!("target/debug/hiroute{suffix}")),
        hirouted: workspace.join(format!("target/debug/hirouted{suffix}")),
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn assert_contract(scenario: &ScenarioV1, boundary: &BoundaryV1) {
    assert_eq!(
        scenario.schema,
        "hiroute.pre-gateway-compute-routing-scenario/v1"
    );
    assert_eq!(scenario.process, "PROCESS-25026");
    assert_eq!(
        scenario.scenario_id,
        "pre-gateway-compute-routing-production-boundary"
    );
    assert_eq!(scenario.command_lifecycle, "released");
    assert_eq!(scenario.compute_focused_state, "green");
    assert_eq!(scenario.routing_preview_focused_state, "green");
    assert_eq!(scenario.routing_apply_focused_state, "expected_red");
    assert_eq!(scenario.routing_apply_error, "GATEWAY_UNAVAILABLE");
    assert_eq!(scenario.production_subprocess_state, "green");
    assert_eq!(
        scenario.production_catalog_source,
        "daemon_embedded_current"
    );
    assert_eq!(scenario.storage_catalog_tampering, "ignored");
    assert_eq!(scenario.daemon_stop, "owned_finite_stop");
    assert_eq!(scenario.test_process_expected_exit, 0);

    assert_eq!(
        boundary.schema,
        "hiroute.pre-gateway-compute-routing-boundary/v1"
    );
    assert_eq!(boundary.process, scenario.process);
    assert_eq!(
        boundary.production_catalog_trust,
        "compiled_client_bundled_manifest_only"
    );
    assert!(!boundary.fixture_trust_is_production_fallback);
    assert!(boundary.cli_direct_business_dependencies.is_empty());
    assert_eq!(boundary.local_control_operations.len(), 6);
    assert_eq!(boundary.application_entrypoints, 1);
    assert_eq!(boundary.preview_database_writes, 0);
    assert!(boundary.compute_apply_operation_journaled);
    assert_eq!(boundary.routing_apply_operation_writes, 0);
    assert_eq!(boundary.routing_apply_plan_writes, 0);
    assert_eq!(boundary.routing_apply_publication_writes, 0);
    assert!(!boundary.routing_apply_capability_consumed);
    assert_eq!(boundary.final_gateway_installer, "not_injected");
    assert_eq!(
        boundary.release_artifact_generation_owner,
        "convergence_owner"
    );
    assert_eq!(boundary.public_command_promotion_owner, "TASK-142003");
}
