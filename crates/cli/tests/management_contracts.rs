//! Installed binary contracts: no checkout, service or network is needed for discovery.
use serde_json::{Value, json};
use std::{
    io::Write,
    process::{Command, Stdio},
};

fn invoke(args: &[&str], input: &[u8]) -> std::process::Output {
    let root = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_hiroute"))
        .args(args)
        .current_dir(root.path())
        .env("HIROUTE_RUNTIME_DIR", root.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}
fn refs_resolve(value: &Value, root: &Value) {
    match value {
        Value::Object(object) => {
            if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
                let pointer = reference
                    .strip_prefix('#')
                    .expect("installed schema has an external dependency");
                assert!(root.pointer(pointer).is_some(), "unresolved {reference}");
            }
            for child in object.values() {
                refs_resolve(child, root);
            }
        }
        Value::Array(values) => {
            for child in values {
                refs_resolve(child, root);
            }
        }
        _ => {}
    }
}

#[test]
fn family_help_is_navigable_and_does_not_promote_unreleased_commands() {
    for family in [
        &["compute"][..],
        &["compute", "connection"],
        &["routing"],
        &["agents"],
        &["agents", "connect"],
        &["agents", "restore"],
        &["operations"],
        &["decision", "services"],
    ] {
        let mut args = family.to_vec();
        args.push("--help");
        let output = invoke(&args, b"");
        assert!(output.status.success(), "{args:?}");
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains("Commands") && text.contains("--include-schema"));
        assert!(!text.contains("setup preview"));
        if family == ["decision", "services"] {
            assert!(!text.contains("  preview\n"));
            assert!(text.contains("options [--provider ID]"));
        }
    }
    assert!(!invoke(&["setup", "--help"], b"").status.success());
}

#[test]
fn installed_full_schemas_are_offline_complete_and_opt_in() {
    for id in [
        "compute.scan",
        "compute.connection.prepare",
        "compute.connection.options",
        "compute.connection.preview",
        "compute.connection.apply",
        "compute.connection.authorize",
        "routing.options",
        "routing.list",
        "routing.show",
        "routing.preview",
        "routing.apply",
        "agents.scan",
        "agents.connect.preview",
        "agents.connect.apply",
        "agents.connect.status",
        "agents.restore.preview",
        "agents.restore.apply",
        "agents.check",
        "operations.find",
        "operations.get",
        "decision.services.list",
        "decision.services.apply",
        "decision.services.test",
        "sessions.list",
        "sessions.show",
        "sessions.receipt",
        "sessions.status",
        "observation.plan-quality.samples",
    ] {
        let output = invoke(
            &["schema", "show", "--command-id", id, "--include-schema"],
            b"",
        );
        assert!(
            output.status.success(),
            "{id}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(envelope["data"]["descriptor"]["command_id"], id);
        for direction in ["request", "response"] {
            let schema = &envelope["data"]["schemas"][direction];
            assert!(schema["$schema"].is_string(), "{id} {direction}");
            refs_resolve(schema, schema);
        }
        let legacy = invoke(&["schema", "show", "--command-id", id], b"");
        let legacy: Value = serde_json::from_slice(&legacy.stdout).unwrap();
        assert_eq!(
            legacy["data"], envelope["data"]["descriptor"],
            "{id} changed legacy output"
        );
    }
}

#[test]
fn decision_choices_work_offline_without_claiming_saved_availability() {
    let output = invoke(&["decision", "services", "options"], b"");
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    let options = &value["data"];
    assert_eq!(options["saved_services_state"], "unavailable");
    assert_eq!(options["saved_services"], Value::Null);
    assert_eq!(options["saved_connection_test_state"], "unknown_not_probed");
    let providers = options["providers"].as_array().unwrap();
    assert_eq!(providers.len(), 6);
    let openrouter = providers.iter().find(|p| p["id"] == "openrouter").unwrap();
    assert_eq!(openrouter["model"], "typesafe/jev-1.13");
    assert!(options["effects"].as_array().unwrap().is_empty());
    let invalid = invoke(
        &[
            "decision",
            "services",
            "options",
            "--provider",
            "ordinary-chat",
        ],
        b"",
    );
    assert_eq!(invalid.status.code(), Some(2));
    let selected = invoke(
        &[
            "decision",
            "services",
            "options",
            "--provider",
            "bailian-workspace",
        ],
        b"",
    );
    let selected: Value = serde_json::from_slice(&selected.stdout).unwrap();
    assert!(
        selected["data"]["selected_provider"]["required_new_connection_flags"]
            .as_array()
            .unwrap()
            .contains(&json!("--endpoint"))
    );
    let custom = invoke(
        &["decision", "services", "options", "--provider", "custom"],
        b"",
    );
    let custom: Value = serde_json::from_slice(&custom.stdout).unwrap();
    assert!(custom["data"]["credential_step"]["arguments"].is_null());
    assert!(
        !custom["data"]["credential_step"]["required_choices"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn examples_reach_transport_and_wrong_wrappers_are_rejected_locally() {
    for (id, path) in [
        ("routing.options", vec!["routing", "options"]),
        (
            "compute.connection.options",
            vec!["compute", "connection", "options"],
        ),
        (
            "decision.services.apply",
            vec!["decision", "services", "apply"],
        ),
    ] {
        let schemas = hiroute_application_api::command_schemas(id).unwrap();
        for example in schemas.examples {
            let mut args = path.clone();
            args.push("--request-stdin");
            let output = invoke(&args, &serde_json::to_vec(&example).unwrap());
            assert_eq!(
                output.status.code(),
                Some(6),
                "{id}: {}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
    }
    let output = invoke(
        &["routing", "preview", "--request-stdin"],
        br#"{"schema":"hiroute.plan-content-change/v2","editor":{}}"#,
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("routing.preview --include-schema")
    );
}

#[test]
fn detailed_errors_are_bounded_redacted_and_keep_real_exit_and_complete_stdout() {
    let cases = [
        (
            br#"{"authoring":{"mode":"SECRET_MARKER_NOT_AN_ENUM"}}"#.as_slice(),
            "invalid_enum",
        ),
        (
            br#"{"authoring":{"candidates":[{"binding_id":7,"reasoning":"SECRET_MARKER"}]}}"#,
            "invalid_type",
        ),
        (
            br#"{"authoring":{"candidates":[{"reasoning":null}]}}"#,
            "missing_field",
        ),
        (
            br#"{"authoring":{"SECRET_MARKER":"SECRET_MARKER"}}"#,
            "unknown_field",
        ),
        (
            br#"{"authoring":{},"authoring":{"SECRET_MARKER":"SECRET_MARKER"}}"#,
            "duplicate_field",
        ),
        (b"{} {} SECRET_MARKER", "trailing_json"),
    ];
    for (input, category) in cases {
        for rich in [false, true] {
            let mut args = vec!["routing", "options", "--request-stdin"];
            if rich {
                args.push("--error-details");
            }
            let output = invoke(&args, input);
            assert_eq!(output.status.code(), Some(2));
            let result: Value = serde_json::from_slice(&output.stdout)
                .expect("whole stdout must contain exactly one JSON document");
            assert_eq!(result["error"]["code"], "INVALID_ARGUMENTS");
            assert_eq!(result.get("data").is_some(), rich);
            if rich {
                assert_eq!(result["data"]["diagnostic"]["category"], category);
            }
            for bytes in [&output.stdout, &output.stderr] {
                assert!(bytes.len() < 4096);
                assert!(!String::from_utf8_lossy(bytes).contains("SECRET_MARKER"));
            }
        }
    }
    let output = invoke(
        &["routing", "options", "--request-stdin", "--error-details"],
        &serde_json::to_vec(&json!({"authoring":{"candidates":[{"reasoning":null}]}})).unwrap(),
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        value["data"]["diagnostic"]["path"],
        "$.authoring.candidates[*].binding_id"
    );
}
