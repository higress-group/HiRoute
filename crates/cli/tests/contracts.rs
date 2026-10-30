use std::path::Path;
use std::process::Command;

use hiroute_application_api::{generated_contract_files, planned_commands, release_manifest};

#[test]
fn checked_in_contracts_are_exact_generator_output() {
    let output = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/cli");
    for file in generated_contract_files() {
        let actual = std::fs::read_to_string(output.join(file.relative_path))
            .unwrap_or_else(|error| panic!("missing generated {}: {error}", file.relative_path));
        assert_eq!(actual, file.contents, "{} is stale", file.relative_path);
    }
}

#[test]
fn each_p0_leaf_has_stable_positive_and_negative_plans() {
    let commands = planned_commands();
    assert_eq!(commands.len(), 113); // Includes versioned decision connections and safe source queries.
    for command in commands {
        assert!(command.positive.scenario_id.ends_with(".positive"));
        assert!(command.negative.scenario_id.ends_with(".negative"));
        assert_ne!(command.positive.scenario_id, command.negative.scenario_id);
    }
}

#[test]
fn release_manifest_is_a_strict_subset_of_planned_registry() {
    let release = release_manifest();
    assert_eq!(release.visibility, "release");
    assert_eq!(
        release
            .commands
            .iter()
            .map(|command| command.command_id.as_str())
            .collect::<Vec<_>>(),
        [
            "console.open",
            "console.token.create",
            "console.token.list",
            "console.token.revoke",
            "schema.list",
            "schema.show",
            "operations.find",
            "system.status",
            "agents.scan",
            "agents.list",
            "agents.previous-routing.preview",
            "agents.connect.preview",
            "agents.connect.apply",
            "agents.connect.status",
            "agents.restore.preview",
            "agents.restore.apply",
            "agents.check",
            "agent.launch",
            "compute.scan",
            "compute.list",
            "compute.show",
            "compute.connection.prepare",
            "compute.connection.options",
            "compute.connection.preview",
            "compute.connection.apply",
            "compute.connection.authorize",
            "compute.connection.login",
            "compute.connection.test",
            "routing.options",
            "worker.executors",
            "worker.dependencies.discover",
            "worker.dependencies.select",
            "worker.plans",
            "worker.list",
            "worker.exec",
            "worker.status",
            "worker.wait",
            "worker.result",
            "worker.read",
            "worker.continue",
            "worker.cancel",
            "worker.cleanup.confirm",
            "routing.list",
            "routing.show",
            "routing.preview",
            "routing.apply",
            "decision.services.list",
            "decision.services.apply",
            "decision.sources.list",
            "decision.connections.list",
            "decision.connections.apply",
            "decision.connections.test",
            "decision.services.test",
            "models.feed.status",
            "models.feed.refresh",
            "models.feed.settings.apply",
            "models.feed.remove.preview",
            "models.feed.remove.apply",
            "models.availability",
            "compute.defaults.get",
            "compute.defaults.preview",
            "models.metadata.list",
            "models.metadata.get",
            "models.show",
            "prices.effective",
            "prices.override.preview",
            "prices.override.apply",
            "operations.get",
            "sessions.list",
            "sessions.show",
            "sessions.receipt",
            "sessions.status",
            "value.show",
            "sessions.timeline",
            "observation.plan-trust",
            "observation.plan-trust.replay",
            "observation.plan-trust.suggestions",
            "observation.storage.show",
            "observation.storage.set",
            "observation.plan-quality.samples",
        ]
    );
}

#[test]
fn released_worker_dependency_selection_declares_both_json_sources() {
    let command = release_manifest()
        .commands
        .into_iter()
        .find(|command| command.command_id == "worker.dependencies.select")
        .unwrap();
    assert_eq!(command.stdin_channels, ["request_stdin", "request_file"]);
    assert!(!command.help.usage.contains("capability"));
    assert!(!command.help.effects.contains("capability"));
    assert!(!command.help_document().contains("--capability-fd"));
}

#[test]
fn packaged_management_skill_explains_discovery_dependency_selection_and_recovery() {
    let skill = include_str!("../../../assets/skills/hiroute-management/SKILL.md");
    assert!(
        skill.len() <= 15_360,
        "leave room within the unchanged 16 KiB installer limit"
    );
    for required in [
        "family/leaf --help",
        "schema show --command-id",
        "worker dependencies discover --harness",
        "selection_revisions[].revision",
        "expected_selection_revision",
        "not a CLI/package version",
        "identical request/key",
        "hiroute value show --view summary",
        "--order-by cost-desc",
        "--view routing --analysis-scope",
        "--view conversation --analysis-scope",
        "continuation_boundary",
        "turn_context_reset",
        "--restore-from",
        "complete baseline",
        "outside chat",
    ] {
        assert!(
            skill.contains(required),
            "management Skill is missing {required}"
        );
    }
}

#[test]
fn packaged_headless_docs_distinguish_host_and_application_discovery() {
    let docs = include_str!("../../../docs/standalone-cli.md");
    for required in [
        "Host management commands",
        "hiroute --help",
        "hiroute service --help",
        "hiroute gateway --help",
        "hiroute protected-input --help",
        "hiroute subscription-proxy --help",
        "Application/Local Control",
        "hiroute schema list --output json",
        "hiroute schema show --command-id compute.connection.test --include-schema --output json",
    ] {
        assert!(
            docs.contains(required),
            "standalone docs are missing {required}"
        );
    }
}

#[test]
fn real_hiroute_help_exposes_both_public_contract_layers_without_a_daemon() {
    let root = Command::new(env!("CARGO_BIN_EXE_hiroute"))
        .arg("--help")
        .output()
        .expect("launch real hiroute root help");
    assert_eq!(root.status.code(), Some(0));
    assert!(root.stderr.is_empty());
    let root = String::from_utf8(root.stdout).unwrap();
    assert!(root.contains("Host management"));
    assert!(root.contains("Application/Local Control"));
    assert!(root.contains("  subscription-proxy"));
    assert!(root.contains("hiroute subscription-proxy --help"));

    for family in [
        "service",
        "gateway",
        "protected-input",
        "subscription-proxy",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_hiroute"))
            .args([family, "--help"])
            .output()
            .unwrap_or_else(|error| panic!("launch real hiroute {family} help: {error}"));
        assert_eq!(output.status.code(), Some(0), "{family} help failed");
        assert!(output.stderr.is_empty());
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.contains("Usage"));
        assert!(stdout.contains(family));
        if family == "subscription-proxy" {
            assert!(stdout.contains("--mode inherit|direct|manual"));
            assert!(stdout.contains("hiroute service restart"));
        }
    }
}

#[test]
fn real_worker_help_requires_the_original_key_and_request_after_uncertain_delivery() {
    let output = Command::new(env!("CARGO_BIN_EXE_hiroute"))
        .args(["worker", "exec", "--help"])
        .output()
        .expect("launch real Worker help without a daemon");
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("without a local receipt"));
    assert!(help.contains("original key and the identical request"));
    assert!(!help.contains("A private receipt is saved before transmission"));
}

#[test]
fn real_hiroute_launcher_emits_the_same_typed_manifest() {
    let cases: &[(&[&str], i32, &str)] = &[
        (
            &["schema", "list", "--non-interactive", "--output", "json"],
            0,
            "succeeded",
        ),
        (&["schema", "list", "unexpected"], 2, "usage_error"),
        (
            &["schema", "show", "--command-id", "schema.list"],
            0,
            "succeeded",
        ),
        (
            &["schema", "show", "--command-id", "setup.preview"],
            5,
            "not_found",
        ),
    ];
    let mut first_envelope = None;
    for (arguments, expected_exit, expected_status) in cases {
        let output = Command::new(env!("CARGO_BIN_EXE_hiroute"))
            .args(arguments.iter().copied())
            .output()
            .expect("launch real hiroute binary");
        assert_eq!(output.status.code(), Some(*expected_exit));
        assert!(output.stderr.is_empty());
        assert_eq!(
            output
                .stdout
                .split(|byte| *byte == b'\n')
                .filter(|line| !line.is_empty())
                .count(),
            1
        );
        let envelope: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(envelope["status"], *expected_status);
        first_envelope.get_or_insert(envelope);
    }
    let envelope = first_envelope.unwrap();
    assert_eq!(
        envelope["data"]["descriptor_digest"],
        release_manifest().descriptor_digest.as_str()
    );
}

#[test]
fn real_hiroute_launch_rejects_override_before_transport_and_is_stably_unavailable() {
    let directory = tempfile::tempdir().unwrap();
    let conflict = Command::new(env!("CARGO_BIN_EXE_hiroute"))
        .env("HOME", directory.path())
        .env("HIROUTE_RUNTIME_DIR", directory.path())
        .args([
            "agent",
            "launch",
            "--agent",
            "claude-code",
            "--context",
            "claude-default",
            "--",
            "--base-url=http://127.0.0.1:1",
        ])
        .output()
        .unwrap();
    assert_eq!(conflict.status.code(), Some(3));
    let envelope: serde_json::Value = serde_json::from_slice(&conflict.stdout).unwrap();
    assert_eq!(envelope["error"]["code"], "AGENT_AUTH_PRECEDENCE_CONFLICT");

    let unavailable = Command::new(env!("CARGO_BIN_EXE_hiroute"))
        .env("HOME", directory.path())
        .env("HIROUTE_RUNTIME_DIR", directory.path())
        .args([
            "agent",
            "launch",
            "--agent",
            "claude-code",
            "--context",
            "claude-default",
            "--",
            "--model=other",
            "--print",
            "hello",
        ])
        .output()
        .unwrap();
    assert_eq!(unavailable.status.code(), Some(6));
    let envelope: serde_json::Value = serde_json::from_slice(&unavailable.stdout).unwrap();
    assert_eq!(envelope["error"]["code"], "DAEMON_UNAVAILABLE");
    let stderr = String::from_utf8(unavailable.stderr).unwrap();
    for expected in [
        "hiroute service status",
        "hiroute service start",
        "Desktop",
        "隔离实例",
        "不会自动启动或重放请求",
    ] {
        assert!(
            stderr.contains(expected),
            "recovery hint is missing {expected}"
        );
    }
}

#[cfg(unix)]
#[test]
fn gateway_show_waits_for_native_discovery_and_honors_control_timeout() {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::time::{Duration, Instant};

    use hiroute_application_api::{
        ClientHelloV1, LocalControlWireRequestV2, MachineEnvelopeV2, negotiate_hello,
    };
    use serde_json::json;

    for explicit_timeout in [false, true] {
        let directory = tempfile::tempdir_in("/tmp").unwrap();
        let home = directory.path().canonicalize().unwrap();
        let marker = home.join(".local/share/hiroute/standalone.json");
        std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
        std::fs::write(
            marker,
            serde_json::to_vec(&json!({
                "schema_version": "hiroute.standalone-install/v1", "version": "0.3.1",
                "target": "fixture", "install_root": home.join("install"),
                "service_definition": home.join("service"),
            }))
            .unwrap(),
        )
        .unwrap();
        let runtime = home.join("run");
        std::fs::create_dir_all(runtime.join("hiroute")).unwrap();
        let listener = UnixListener::bind(runtime.join("hiroute/control.sock")).unwrap();
        listener.set_nonblocking(true).unwrap();
        let (finished, completion) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            for operation in ["ListAgents", "GetAgentConnectionStatus"] {
                let deadline = Instant::now() + Duration::from_secs(10);
                let stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            if completion.try_recv().is_ok() {
                                return;
                            }
                            assert!(Instant::now() < deadline, "CLI never requested {operation}");
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("{error}"),
                    }
                };
                // BSD accept inherits the listener's nonblocking mode. This peer uses
                // blocking reads with a deadline while the client starts its handshake.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut stream = BufReader::new(stream);
                let mut line = String::new();
                if !stream.read_line(&mut line).is_ok_and(|count| count > 0) {
                    return;
                }
                let hello: ClientHelloV1 = serde_json::from_str(&line).unwrap();
                let hello = negotiate_hello(&hello).unwrap();
                if writeln!(
                    stream.get_mut(),
                    "{}",
                    serde_json::to_string(&hello).unwrap()
                )
                .is_err()
                {
                    return;
                }
                line.clear();
                if !stream.read_line(&mut line).is_ok_and(|count| count > 0) {
                    return;
                }
                let request: LocalControlWireRequestV2 = serde_json::from_str(&line).unwrap();
                assert_eq!(request.operation_id, operation);
                // Native executable probes legitimately exceed the TCP readiness probe's 750ms.
                std::thread::sleep(Duration::from_secs(1));
                let data = if operation == "ListAgents" {
                    json!({"agents":[{"context_id":"agent-context/pi",
                        "agent_id":"agent_pi_default", "profile_id":"pi-default"}]})
                } else {
                    assert_eq!(request.payload["context_id"], "agent-context/pi");
                    json!({"schema":"hiroute.agent-model-settings-status/v2",
                        "context_id":"agent-context/pi", "state":"configured",
                        "operation_id":null, "operation_state":null, "restore_point_ref":null,
                        "applied_revision":null, "surface_results":[], "model_verified":false})
                };
                let response = MachineEnvelopeV2::succeeded(data, Some(request.request_id));
                if writeln!(
                    stream.get_mut(),
                    "{}",
                    serde_json::to_string(&response).unwrap()
                )
                .is_err()
                {
                    return;
                }
            }
        });
        let mut command = Command::new(env!("CARGO_BIN_EXE_hiroute"));
        command
            .env("HOME", home)
            .env("HIROUTE_RUNTIME_DIR", &runtime)
            .env_remove("XDG_STATE_HOME")
            .env_remove("XDG_DATA_HOME")
            .env_remove("XDG_RUNTIME_DIR")
            .args(["gateway", "show", "--output", "json"]);
        if explicit_timeout {
            command.args(["--timeout", "50ms"]);
        }
        let output = command.output().unwrap();
        let _ = finished.send(());
        server.join().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let update = &response["data"]["agent_update"];
        assert_eq!(update["required"], explicit_timeout);
        assert_eq!(
            update["existing_agent_count"],
            if explicit_timeout {
                json!(null)
            } else {
                json!(1)
            }
        );
    }
}

#[cfg(unix)]
#[test]
fn real_hidden_helper_uses_only_the_owner_raw_endpoint_and_stdout_token_boundary() {
    use std::io::{Read, Write};
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    use std::os::unix::net::UnixListener;

    use hiroute_application_api::{AgentGrantRawRequestV1, HIDDEN_AGENT_GRANT_HELPER_VERB_V1};

    let directory = tempfile::tempdir().unwrap();
    let protected = directory.path().join("hiroute");
    let mut builder = std::fs::DirBuilder::new();
    builder.mode(0o700).create(&protected).unwrap();
    std::fs::set_permissions(&protected, std::fs::Permissions::from_mode(0o700)).unwrap();
    let socket = protected.join("agent-grant-v1.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut frame = Vec::new();
        stream.read_to_end(&mut frame).unwrap();
        let request = AgentGrantRawRequestV1::decode(&frame).unwrap();
        assert_eq!(request.connection_id(), "agent-connection/claude-default");
        stream.write_all(b"Abc_123-xyz\n").unwrap();
    });

    let output = Command::new(env!("CARGO_BIN_EXE_hiroute"))
        .env("HIROUTE_RUNTIME_DIR", directory.path())
        .args([
            HIDDEN_AGENT_GRANT_HELPER_VERB_V1,
            "agent-connection/claude-default",
        ])
        .output()
        .unwrap();
    server.join().unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"Abc_123-xyz\n");
    assert!(output.stderr.is_empty());

    let manifest = release_manifest();
    let public_json = serde_json::to_string(&manifest).unwrap();
    assert!(!public_json.contains(HIDDEN_AGENT_GRANT_HELPER_VERB_V1));
    let help = Command::new(env!("CARGO_BIN_EXE_hiroute"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(
        !String::from_utf8(help.stdout)
            .unwrap()
            .contains(HIDDEN_AGENT_GRANT_HELPER_VERB_V1)
    );
}

#[cfg(unix)]
#[test]
fn real_hidden_helper_failure_has_empty_stdout() {
    use hiroute_application_api::HIDDEN_AGENT_GRANT_HELPER_VERB_V1;

    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_hiroute"))
        .env("HIROUTE_RUNTIME_DIR", directory.path())
        .args([
            HIDDEN_AGENT_GRANT_HELPER_VERB_V1,
            "agent-connection/missing",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(6));
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
}
