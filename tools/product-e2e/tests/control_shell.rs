#![cfg(unix)]

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use hiroute_application_api::{
    ClientHelloV1, LOCAL_CONTROL_SCHEMA_V2, LocalControlWireRequestV2, MACHINE_ENVELOPE_SCHEMA_V2,
    MachineEnvelopeV2, SchemaVersion, ServerHelloV1,
};
use hiroute_product_e2e::control::ControlShellScenarioV1;
use serde::Deserialize;
use serde_json::{Value, json};

const SCENARIO: &str = include_str!("../../../e2e/product/scenarios/control/control-shell.v1.json");
const FIXTURE: &str = include_str!("../../../e2e/product/fixtures/control/setup-default.v1.json");
const GOLDEN: &str =
    include_str!("../../../e2e/product/golden/control/control-shell-boundary.v1.json");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BoundaryGolden {
    schema: String,
    process: String,
    daemon_role: String,
    transport: String,
    application_entrypoints: u8,
    cli_direct_adapter_dependencies: Vec<String>,
    preview_writes: u8,
    normal_product_traffic: u8,
    connectivity_probe_counted_as_product_traffic: bool,
    activity_database_seeded: bool,
    public_command_promotion_owner: String,
    gateway_bound_positive_state: String,
}

struct ProductBinaries {
    hiroute: PathBuf,
    hirouted: PathBuf,
}

struct DaemonGuard(Child);

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn control_shell_uses_real_hiroute_and_hirouted_processes() {
    let scenario: ControlShellScenarioV1 = serde_json::from_str(SCENARIO).unwrap();
    scenario.validate().unwrap();
    let expected = scenario
        .command_states
        .iter()
        .map(|state| (state.command_id.as_str(), state))
        .collect::<BTreeMap<_, _>>();
    let golden: BoundaryGolden = serde_json::from_str(GOLDEN).unwrap();
    assert_boundary(&golden);

    let workspace = workspace_root();
    let binaries = build_product_binaries(&workspace);
    let temporary = tempfile::tempdir().unwrap();
    let storage_root = temporary.path().join("storage");
    let runtime_root = temporary.path().join("runtime");
    let endpoint = runtime_root.join("hiroute/control.sock");
    let (agent_home, agent_path, secret_sentinel) = agent_discovery_fixture(temporary.path());
    let rejected_role_all = Command::new(&binaries.hirouted)
        .args([
            "--role",
            "all",
            "--storage-root",
            storage_root.to_str().unwrap(),
            "--runtime-root",
            runtime_root.to_str().unwrap(),
        ])
        .output()
        .expect("launch uncomposed role=all boundary check");
    assert_eq!(rejected_role_all.status.code(), Some(2));
    assert!(rejected_role_all.stdout.is_empty());
    assert!(!endpoint.exists());

    let daemon = Command::new(&binaries.hirouted)
        .args([
            "--role",
            "control",
            "--storage-root",
            storage_root.to_str().unwrap(),
            "--runtime-root",
            runtime_root.to_str().unwrap(),
        ])
        .env("HOME", &agent_home)
        .env_remove("CODEX_HOME")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env("PATH", agent_path)
        .env_remove("ANTHROPIC_BASE_URL")
        .env_remove("ANTHROPIC_MODEL")
        .env_remove("ANTHROPIC_DEFAULT_OPUS_MODEL")
        .env_remove("ANTHROPIC_AUTH_TOKEN")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("launch production hirouted control role");
    let mut daemon = DaemonGuard(daemon);
    wait_for_endpoint(&endpoint, &mut daemon.0);

    let mut stalled_peer = UnixStream::connect(&endpoint).expect("stalled Local Control peer");
    stalled_peer.write_all(b"{").unwrap();

    let system = run_cli(
        &binaries.hiroute,
        &runtime_root,
        &["system", "status", "--request-id", "equivalent-request"],
    );
    assert_cli(&system, expected["system.status"]);
    assert_eq!(system.envelope["data"]["daemon"], "control_only");
    assert_eq!(
        system.envelope["data"]["gateway"],
        "unavailable:not_composed"
    );
    drop(stalled_peer);

    let desktop = desktop_call(
        &endpoint,
        ClientHelloV1 {
            api_version: LOCAL_CONTROL_SCHEMA_V2,
            machine_schema_version: MACHINE_ENVELOPE_SCHEMA_V2,
            client_name: "hiroute-desktop".to_owned(),
            client_version: env!("CARGO_PKG_VERSION").to_owned(),
        },
        Some(LocalControlWireRequestV2 {
            schema_version: LOCAL_CONTROL_SCHEMA_V2,
            request_id: "equivalent-request".to_owned(),
            operation_id: "GetSystemStatus".to_owned(),
            payload: json!({}),
            protected_grant: None,
        }),
    );
    assert_eq!(desktop["status"], "succeeded");
    assert_eq!(desktop["data"]["daemon"], "control_only");
    assert_eq!(desktop["data"]["gateway"], "unavailable:not_composed");

    for (command, operation) in [
        (["agents", "scan"], "ScanAgents"),
        (["agents", "list"], "ListAgents"),
    ] {
        let observed = run_cli(&binaries.hiroute, &runtime_root, &command);
        let command_id = format!("{}.{}", command[0], command[1]);
        assert_cli(&observed, expected[command_id.as_str()]);
        assert_eq!(
            observed.envelope["data"]["agents"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        let internal = desktop_call(
            &endpoint,
            ClientHelloV1 {
                api_version: LOCAL_CONTROL_SCHEMA_V2,
                machine_schema_version: MACHINE_ENVELOPE_SCHEMA_V2,
                client_name: "hiroute-desktop".to_owned(),
                client_version: env!("CARGO_PKG_VERSION").to_owned(),
            },
            Some(LocalControlWireRequestV2 {
                schema_version: LOCAL_CONTROL_SCHEMA_V2,
                request_id: format!("internal-{operation}"),
                operation_id: operation.to_owned(),
                payload: json!({}),
                protected_grant: None,
            }),
        );
        assert_eq!(internal["status"], "succeeded");
        let agents = internal["data"]["agents"].as_array().unwrap();
        assert_eq!(agents.len(), 2);
        let claude = agents
            .iter()
            .find(|agent| agent["agent_id"] == "agent_claude_default")
            .unwrap();
        assert_eq!(claude["supported"], true);
        assert_eq!(
            claude["registered_configuration"]["connection_option_id"],
            "zhipu.coding-plan.cn.v1"
        );
        assert_eq!(
            claude["discovered_credential"]["field_selector"],
            "env.ANTHROPIC_AUTH_TOKEN"
        );
        let encoded = internal.to_string();
        assert!(!encoded.contains(secret_sentinel));
        assert!(!encoded.contains(agent_home.to_string_lossy().as_ref()));
    }

    for (command_id, arguments) in [
        ("sessions.list", vec!["sessions", "list"]),
        ("sessions.status", vec!["sessions", "status"]),
        ("value.show", vec!["value", "show"]),
        (
            "operations.get",
            vec!["operations", "get", "op_00000000000000000000000000000000"],
        ),
        (
            "operations.cancel",
            vec![
                "operations",
                "cancel",
                "op_00000000000000000000000000000000",
                "--idempotency-key",
                "control-shell-cancel-1",
            ],
        ),
        ("sessions.show", vec!["sessions", "show", "session-missing"]),
        (
            "sessions.receipt",
            vec!["sessions", "receipt", "receipt-missing"],
        ),
    ] {
        let observed = run_cli(&binaries.hiroute, &runtime_root, &arguments);
        assert_cli(&observed, expected[command_id]);
    }

    let before = durable_store_snapshot(&storage_root);
    let preview = run_cli(&binaries.hiroute, &runtime_root, &["setup", "preview"]);
    assert_cli(&preview, expected["setup.preview"]);
    assert_eq!(durable_store_snapshot(&storage_root), before);

    let fixture_path = temporary.path().join("setup.json");
    std::fs::write(&fixture_path, FIXTURE).unwrap();
    let accepted = "sha256:0000000000000000000000000000000000000000000000000000000000000000";
    let revision = "0";
    let apply = run_cli_with_stdin(
        &binaries.hiroute,
        &runtime_root,
        &[
            "setup",
            "apply",
            "--spec-fd",
            "0",
            "--accept-digest",
            accepted,
            "--expected-revision",
            revision,
            "--idempotency-key",
            "control-shell-apply-1",
        ],
        File::open(&fixture_path).unwrap(),
    );
    assert_cli(&apply, expected["setup.apply"]);
    assert_eq!(durable_store_snapshot(&storage_root), before);

    let capability_path = temporary.path().join("invalid-capability");
    std::fs::write(&capability_path, "invalid-control-capability\n").unwrap();
    let wrong_capability = run_cli_with_capability_fd(
        &binaries.hiroute,
        &runtime_root,
        &fixture_path,
        &capability_path,
        accepted,
        revision,
    );
    assert_cli(&wrong_capability, expected["setup.apply"]);
    assert!(
        !wrong_capability
            .envelope
            .to_string()
            .contains("invalid-control-capability")
    );
    assert_eq!(durable_store_snapshot(&storage_root), before);

    let incompatible = desktop_call(
        &endpoint,
        ClientHelloV1 {
            api_version: SchemaVersion::new(3, 0),
            machine_schema_version: MACHINE_ENVELOPE_SCHEMA_V2,
            client_name: "hiroute-desktop".to_owned(),
            client_version: env!("CARGO_PKG_VERSION").to_owned(),
        },
        None,
    );
    assert_eq!(incompatible["status"], "usage_error");
    assert_eq!(incompatible["error"]["code"], "SCHEMA_INCOMPATIBLE");

    let unprotected_skill = desktop_call(
        &endpoint,
        ClientHelloV1 {
            api_version: LOCAL_CONTROL_SCHEMA_V2,
            machine_schema_version: MACHINE_ENVELOPE_SCHEMA_V2,
            client_name: "hiroute-skill".to_owned(),
            client_version: env!("CARGO_PKG_VERSION").to_owned(),
        },
        Some(LocalControlWireRequestV2 {
            schema_version: LOCAL_CONTROL_SCHEMA_V2,
            request_id: "unprotected-content".to_owned(),
            operation_id: "GetSession".to_owned(),
            payload: json!({"id": "session-missing", "content": "messages"}),
            protected_grant: None,
        }),
    );
    assert_eq!(unprotected_skill["status"], "denied");
    assert_eq!(unprotected_skill["error"]["code"], "CAPABILITY_DENIED");
}

struct CliObservation {
    process_exit: i32,
    envelope: Value,
    stdout_lines: usize,
    stderr: Vec<u8>,
}

fn run_cli(binary: &Path, runtime_root: &Path, arguments: &[&str]) -> CliObservation {
    observe(
        Command::new(binary)
            .args(arguments)
            .args(["--output", "json", "--non-interactive"])
            .env("HIROUTE_RUNTIME_DIR", runtime_root)
            .output()
            .expect("launch production hiroute"),
    )
}

fn run_cli_with_stdin(
    binary: &Path,
    runtime_root: &Path,
    arguments: &[&str],
    stdin: File,
) -> CliObservation {
    observe(
        Command::new(binary)
            .args(arguments)
            .args(["--output", "json", "--non-interactive"])
            .env("HIROUTE_RUNTIME_DIR", runtime_root)
            .stdin(Stdio::from(stdin))
            .output()
            .expect("launch production hiroute with protected spec fd"),
    )
}

fn run_cli_with_capability_fd(
    binary: &Path,
    runtime_root: &Path,
    spec_path: &Path,
    capability_path: &Path,
    digest: &str,
    revision: &str,
) -> CliObservation {
    observe(
        Command::new("sh")
            .arg("-c")
            .arg(
                "exec \"$1\" setup apply --spec-fd 4 --capability-fd 3 \\
                 --accept-digest \"$4\" --expected-revision \"$5\" \\
                 --idempotency-key control-shell-wrong-capability \\
                 --output json --non-interactive 3<\"$2\" 4<\"$3\"",
            )
            .arg("control-shell-capability-wrapper")
            .arg(binary)
            .arg(capability_path)
            .arg(spec_path)
            .arg(digest)
            .arg(revision)
            .env("HIROUTE_RUNTIME_DIR", runtime_root)
            .output()
            .expect("launch production hiroute with protected capability fd"),
    )
}

fn observe(output: Output) -> CliObservation {
    CliObservation {
        process_exit: output.status.code().expect("hiroute was not signaled"),
        envelope: serde_json::from_slice(&output.stdout).expect("one machine envelope"),
        stdout_lines: output
            .stdout
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .count(),
        stderr: output.stderr,
    }
}

fn assert_cli(
    observed: &CliObservation,
    expected: &hiroute_product_e2e::control::ControlCommandStateV1,
) {
    assert_eq!(observed.process_exit, i32::from(expected.expected_exit));
    assert_eq!(observed.stdout_lines, 1);
    assert!(observed.stderr.is_empty());
    assert_eq!(observed.envelope["status"], expected.expected_status);
    match expected.expected_error_code.as_deref() {
        Some(code) => assert_eq!(observed.envelope["error"]["code"], code),
        None => assert!(observed.envelope["error"].is_null()),
    }
}

fn desktop_call(
    endpoint: &Path,
    hello: ClientHelloV1,
    request: Option<LocalControlWireRequestV2>,
) -> Value {
    let mut stream = UnixStream::connect(endpoint).expect("Desktop Local Control connection");
    serde_json::to_writer(&mut stream, &hello).unwrap();
    stream.write_all(b"\n").unwrap();
    stream.flush().unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let value: Value = serde_json::from_str(&line).unwrap();
    if serde_json::from_value::<ServerHelloV1>(value.clone()).is_err() {
        return value;
    }
    let request = request.expect("compatible hello has a request");
    serde_json::to_writer(&mut stream, &request).unwrap();
    stream.write_all(b"\n").unwrap();
    stream.flush().unwrap();
    line.clear();
    reader.read_line(&mut line).unwrap();
    let envelope: MachineEnvelopeV2<Value> = serde_json::from_str(&line).unwrap();
    serde_json::to_value(envelope).unwrap()
}

fn durable_store_snapshot(storage_root: &Path) -> BTreeMap<String, String> {
    fn visit(root: &Path, path: &Path, snapshot: &mut BTreeMap<String, String>) {
        let mut entries = std::fs::read_dir(path)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            if path.strip_prefix(root).unwrap() == Path::new("diagnostics") {
                continue;
            }
            if entry.file_type().unwrap().is_dir() {
                visit(root, &path, snapshot);
            } else {
                snapshot.insert(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    hiroute_application_api::CanonicalDigest::of_bytes(
                        &std::fs::read(path).unwrap(),
                    )
                    .to_string(),
                );
            }
        }
    }

    let mut snapshot = BTreeMap::new();
    visit(storage_root, storage_root, &mut snapshot);
    snapshot
}

fn wait_for_endpoint(endpoint: &Path, child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if endpoint
            .metadata()
            .is_ok_and(|metadata| metadata.file_type().is_socket())
        {
            return;
        }
        if let Some(status) = child.try_wait().unwrap() {
            let mut stderr = String::new();
            if let Some(mut stream) = child.stderr.take() {
                let _ = stream.read_to_string(&mut stderr);
            }
            panic!("hirouted exited before readiness: {status}; stderr: {stderr}");
        }
        assert!(Instant::now() < deadline, "hirouted readiness timed out");
        std::thread::sleep(Duration::from_millis(25));
    }
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
        .expect("build production subprocesses");
    assert!(status.success(), "production subprocess build failed");
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

fn agent_discovery_fixture(root: &Path) -> (PathBuf, std::ffi::OsString, &'static str) {
    const SECRET: &str = "p25019-subprocess-secret-must-not-cross-local-control";
    let (home, bin) = hiroute_product_e2e::smoke::fixtures::discovery(root, SECRET).unwrap();
    let path = std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    )))
    .unwrap();
    (home, path, SECRET)
}

fn assert_boundary(golden: &BoundaryGolden) {
    assert_eq!(golden.schema, "hiroute.control-shell-boundary/v1");
    assert_eq!(golden.process, "PROCESS-25016");
    assert_eq!(golden.daemon_role, "control_only");
    assert_eq!(golden.transport, "owner_only_uds_peer_uid_v1");
    assert_eq!(golden.application_entrypoints, 1);
    assert!(golden.cli_direct_adapter_dependencies.is_empty());
    assert_eq!(golden.preview_writes, 0);
    assert_eq!(golden.normal_product_traffic, 0);
    assert!(!golden.connectivity_probe_counted_as_product_traffic);
    assert!(!golden.activity_database_seeded);
    assert_eq!(golden.public_command_promotion_owner, "TASK-142003");
    assert_eq!(golden.gateway_bound_positive_state, "green");
}
