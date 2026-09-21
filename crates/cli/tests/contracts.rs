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
    assert_eq!(commands.len(), 83); // Full staged surface plus released public Worker lifecycle.
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
            "schema.list",
            "schema.show",
            "operations.find",
            "system.status",
            "agents.scan",
            "agents.list",
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
            "compute.connection.options",
            "compute.connection.preview",
            "compute.connection.apply",
            "compute.connection.authorize",
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
            "routing.list",
            "routing.show",
            "routing.preview",
            "routing.apply",
            "models.show",
            "operations.get",
            "sessions.list",
            "sessions.show",
            "sessions.receipt",
            "sessions.status",
            "value.show",
        ]
    );
}

#[test]
fn released_worker_dependency_selection_declares_only_its_request_stdin_channel() {
    let command = release_manifest()
        .commands
        .into_iter()
        .find(|command| command.command_id == "worker.dependencies.select")
        .unwrap();
    assert_eq!(command.stdin_channels, ["request_stdin"]);
    assert!(!command.help.usage.contains("capability"));
    assert!(!command.help.effects.contains("capability"));
    assert!(!command.help_document().contains("--capability-fd"));
}

#[test]
fn packaged_management_skill_explains_discovery_dependency_selection_and_recovery() {
    let skill = include_str!("../../../assets/skills/hiroute-management/SKILL.md");
    for required in [
        "hiroute --help",
        "hiroute service --help",
        "hiroute gateway --help",
        "hiroute protected-input --help",
        "Application/Local Control commands",
        "worker dependencies discover --harness",
        "--output json",
        "selection_revisions[].revision",
        "expected_selection_revision",
        "replay that exact JSON unchanged",
        "not a CLI/package version",
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
        "Application/Local Control",
        "hiroute schema list --output json",
        "hiroute schema show --command-id compute.connection.test --output json",
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

    for family in ["service", "gateway", "protected-input"] {
        let output = Command::new(env!("CARGO_BIN_EXE_hiroute"))
            .args([family, "--help"])
            .output()
            .unwrap_or_else(|error| panic!("launch real hiroute {family} help: {error}"));
        assert_eq!(output.status.code(), Some(0), "{family} help failed");
        assert!(output.stderr.is_empty());
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.contains("Usage"));
        assert!(stdout.contains(family));
    }
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
