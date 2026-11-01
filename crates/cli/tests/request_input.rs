use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

fn invoke(root: &Path, arguments: &[&str], input: &[u8]) -> (i32, Value) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_hiroute"))
        .args(arguments)
        .env("HIROUTE_RUNTIME_DIR", root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let _ = child.stdin.take().unwrap().write_all(input);
    let output = child.wait_with_output().unwrap();
    let envelope = serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "invalid CLI response: {}",
            String::from_utf8_lossy(&output.stdout)
        )
    });
    (output.status.code().unwrap(), envelope)
}

fn facts() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schema":"hiroute.observation.query/v2",
        "intent":{"view":"facts","query":{
            "request_id":"request/one", "limit":100, "cursor":null
        }}
    }))
    .unwrap()
}

#[test]
fn worker_dependency_input_details_explain_the_harness_tuple_without_echoing_paths() {
    let root = tempfile::tempdir().unwrap();
    let private_path = root
        .path()
        .join("PRIVATE_COMPONENT_MUST_NOT_APPEAR")
        .display()
        .to_string();
    for (request, expected_path) in [
        (
            json!({"harness":"pi", "node_path":private_path, "expected_selection_revision":0}),
            "$.cli_path",
        ),
        (
            json!({"harness":"pi", "cli_path":private_path, "expected_selection_revision":0}),
            "$.node_path",
        ),
        (
            json!({"harness":"pi", "cli_path":private_path, "node_path":private_path, "adapter_path":private_path, "expected_selection_revision":0}),
            "$.adapter_path",
        ),
        (
            json!({"harness":"claude_code", "cli_path":private_path, "expected_selection_revision":0}),
            "$.adapter_path",
        ),
        (
            json!({"harness":"opencode", "cli_path":private_path, "node_path":private_path, "expected_selection_revision":0}),
            "$.node_path",
        ),
    ] {
        let bytes = serde_json::to_vec(&request).unwrap();
        let (exit, envelope) = invoke(
            root.path(),
            &[
                "worker",
                "dependencies",
                "select",
                "--request-stdin",
                "--error-details",
                "--output",
                "json",
            ],
            &bytes,
        );
        assert_eq!(exit, 2, "{envelope}");
        assert_eq!(
            envelope["data"]["schema"],
            "hiroute.cli-input-diagnostic/v1"
        );
        assert_eq!(envelope["data"]["diagnostic"]["path"], expected_path);
        assert!(
            !envelope
                .to_string()
                .contains("PRIVATE_COMPONENT_MUST_NOT_APPEAR")
        );
        let (exit, legacy) = invoke(
            root.path(),
            &[
                "worker",
                "dependencies",
                "select",
                "--request-stdin",
                "--output",
                "json",
            ],
            &bytes,
        );
        assert_eq!(exit, 2);
        assert!(legacy.get("data").is_none());
        assert_eq!(legacy["error"]["code"], "INVALID_ARGUMENTS");
    }
}

#[test]
fn unknown_options_never_become_resource_ids_or_reach_transport() {
    let root = tempfile::tempdir().unwrap();
    for path in [
        vec!["sessions", "receipt"],
        vec!["sessions", "show"],
        vec!["routing", "show"],
        vec!["compute", "show"],
        vec!["agents", "connect", "status"],
        vec!["agents", "check"],
        vec!["operations", "get"],
    ] {
        for invalid_id in ["--unknown", "-x", ""] {
            let mut args = path.clone();
            args.extend([invalid_id, "--output", "json"]);
            let (exit, envelope) = invoke(root.path(), &args, b"");
            assert_eq!(exit, 2, "{args:?}: {envelope}");
            assert_eq!(envelope["error"]["code"], "INVALID_ARGUMENTS");
        }
    }
}

#[test]
fn facts_file_and_stdin_parse_before_the_same_transport_boundary() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("事实 request.json");
    let input = facts();
    std::fs::write(&path, &input).unwrap();
    for (args, stdin) in [
        (
            vec![
                "sessions",
                "receipt",
                "--request-file",
                path.to_str().unwrap(),
                "--output",
                "json",
            ],
            b"malformed unused stdin".as_slice(),
        ),
        (
            vec!["sessions", "receipt", "--request-stdin", "--output", "json"],
            input.as_slice(),
        ),
        (
            vec!["sessions", "receipt", "receipt/one", "--output", "json"],
            b"ignored legacy stdin".as_slice(),
        ),
    ] {
        let (exit, envelope) = invoke(root.path(), &args, stdin);
        assert_eq!(exit, 6, "{args:?}: {envelope}");
        assert_eq!(envelope["error"]["code"], "DAEMON_UNAVAILABLE");
    }
}

#[test]
fn both_sources_reject_wrong_intents_unknown_fields_and_ambiguous_json() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("request.json");
    for bytes in [
        br#"{"schema":"hiroute.observation.query/v2","intent":{"view":"facts","query":{"request_id":"one","request_id":"two","limit":100}}}"#.as_slice(),
        br#"{"schema":"hiroute.observation.query/v2","intent":{"view":"facts","query":{"request_id":"one","limit":100,"secret":"reject"}}}"#,
        br#"{"schema":"hiroute.observation.query/v2","intent":{"view":"status","query":{}}}"#,
        b"{} {}", b"", b"\xff", b"{",
    ] {
        std::fs::write(&path, bytes).unwrap();
        for (args, stdin) in [
            (vec!["sessions", "receipt", "--request-file", path.to_str().unwrap()], b"unused".as_slice()),
            (vec!["sessions", "receipt", "--request-stdin"], bytes),
        ] {
            let (exit, envelope) = invoke(root.path(), &args, stdin);
            assert_eq!(exit, 2, "{args:?}: {envelope}");
            assert_eq!(envelope["error"]["code"], "INVALID_ARGUMENTS");
        }
    }
}

#[test]
fn typed_requests_reject_nested_unknowns_and_keep_declared_optional_and_dynamic_fields() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("typed.json");
    let agent = json!({"spec":{
        "schema_version":{"major":2,"minor":0},
        "context_id":"agent-context/codex/test",
        "model":{"intent":"restore","restore_point_ref":"model-restore/original"},
        "collaboration":{"intent":"keep"},
        "access_token":{"intent":"keep"},
        "restore_native_model":null,"protected_native_model_ids":[]
    }});
    let decision = json!({
        "schema_version":{"major":1,"minor":0},
        "spec":{
            "schema_version":{"major":1,"minor":0},
            "command_id":"decision.services.apply","resource_id":null,
            "desired_state":{"arbitrary":{"intent":"keep","typo":1}}
        }
    });
    let mut cases = vec![
        (vec!["agents", "connect", "preview"], agent.clone(), 6),
        (vec!["decision", "services", "apply"], decision.clone(), 6),
    ];
    let mut positional_spec = decision.clone();
    positional_spec["schema_version"] = json!([1, 0]);
    positional_spec["spec"] = json!([
        {"major":1,"minor":0}, "decision.services.apply", null, {"arbitrary":1}
    ]);
    cases.push((
        vec!["decision", "services", "apply"],
        positional_spec.clone(),
        6,
    ));
    positional_spec["spec"][0]["typo"] = Value::Null;
    cases.push((vec!["decision", "services", "apply"], positional_spec, 2));
    let mut authorization = json!({"action":"result","operation":{
        "operation_id":"operation/one","state":"succeeded","sequence":1,"cancellable":false
    }});
    cases.push((
        vec!["compute", "connection", "authorize"],
        authorization.clone(),
        6,
    ));
    authorization["operation"]["typo"] = Value::Null;
    cases.push((vec!["compute", "connection", "authorize"], authorization, 2));
    for pointer in [
        "/spec/schema_version/typo",
        "/spec/model/typo",
        "/spec/collaboration/typo",
        "/spec/access_token/typo",
    ] {
        let mut request = agent.clone();
        let (parent, key) = pointer.rsplit_once('/').unwrap();
        request.pointer_mut(parent).unwrap()[key] = Value::Null;
        cases.push((vec!["agents", "connect", "preview"], request, 2));
    }
    let mut keep = agent.clone();
    keep["spec"]["model"] = json!({"intent":"keep","typo":null});
    cases.push((vec!["agents", "connect", "preview"], keep, 2));
    let mut regenerate = agent.clone();
    regenerate["spec"]["access_token"] = json!({"intent":"regenerate","typo":null});
    cases.push((vec!["agents", "connect", "preview"], regenerate, 2));
    let mut apply = agent.clone();
    apply["accept_digest"] = json!(format!("sha256:{}", "0".repeat(64)));
    apply["dependency_digest"] = apply["accept_digest"].clone();
    apply["idempotency_key"] = json!("regression-only");
    apply["expected_revisions"] = json!({"target":0,"dependencies":{"arbitrary":1}});
    apply["login_item"] = Value::Null;
    cases.push((vec!["agents", "connect", "apply"], apply.clone(), 6));
    apply["expected_revisions"]["typo"] = Value::Null;
    cases.push((vec!["agents", "connect", "apply"], apply, 2));
    for pointer in [
        "/schema_version/typo",
        "/spec/schema_version/typo",
        "/spec/typo",
    ] {
        let mut request = decision.clone();
        let (parent, key) = pointer.rsplit_once('/').unwrap();
        request.pointer_mut(parent).unwrap()[key] = Value::Null;
        cases.push((vec!["decision", "services", "apply"], request, 2));
    }
    for (path, request, expected_exit) in cases {
        let bytes = serde_json::to_vec(&request).unwrap();
        std::fs::write(&file, &bytes).unwrap();
        for source in [
            vec!["--request-file", file.to_str().unwrap()],
            vec!["--request-stdin"],
        ] {
            let mut args = path.clone();
            args.extend(source);
            let (exit, envelope) = invoke(root.path(), &args, &bytes);
            assert_eq!(exit, expected_exit, "{args:?} {request}: {envelope}");
            assert_eq!(
                envelope["error"]["code"],
                if expected_exit == 2 {
                    "INVALID_ARGUMENTS"
                } else {
                    "DAEMON_UNAVAILABLE"
                }
            );
        }
    }
}

#[test]
fn conflicting_missing_and_nonregular_request_files_are_usage_errors() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("request.json");
    std::fs::write(&path, facts()).unwrap();
    for options in [
        vec!["--request-file"],
        vec!["--request-file", ""],
        vec!["--request-file", "--request-stdin"],
        vec!["--request-file", "missing-request.json"],
        vec!["--request-file", root.path().to_str().unwrap()],
        vec!["--request-file", path.to_str().unwrap(), "--request-stdin"],
        vec!["--request-stdin", "--request-file", path.to_str().unwrap()],
        vec![
            "--request-file",
            path.to_str().unwrap(),
            "--request-file",
            path.to_str().unwrap(),
        ],
        vec!["--request-stdin", "--request-stdin"],
        vec!["receipt/one", "--request-file", path.to_str().unwrap()],
    ] {
        let mut args = vec!["sessions", "receipt"];
        args.extend(options);
        let (exit, envelope) = invoke(root.path(), &args, &facts());
        assert_eq!(exit, 2, "{args:?}: {envelope}");
        assert_eq!(envelope["error"]["code"], "INVALID_ARGUMENTS");
    }
}

#[test]
fn budget_detail_opt_in_preserves_legacy_inputs_across_both_sources() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("budget-projection.json");
    let digest = format!("sha256:{}", "0".repeat(64));
    let lookup = json!({
        "principal_kind":"interactive_user", "operation_kind":"change",
        "idempotency_key":"test", "accepted_digest":digest
    });
    let publication = json!({
        "change":{
            "schema":"hiroute.plan-content-change/v2",
            "target":{"intent":"create","creation_key":"budget-compatibility"},
            "editor":{
                "schema":"hiroute.plan-editor/v2", "display_name":"Budget compatibility",
                "purpose":"Check request projection", "mode":"fixed_model", "candidates":[],
                "smart":{
                    "economy":[], "primary":[],
                    "judgment":hiroute_domain::JudgmentSettingsV1::default(),
                    "reselect_on_user_message":false, "classifier":{"kind":"local_rules"},
                    "complex_keywords":[]
                },
                "free":{"candidates":[],"primary":[],"primary_fallback":false},
                "delegation_enabled":false,
                "requirements":hiroute_domain::CapabilityRequirementsV1::default(),
                "limits":{"maximum_attempts":1,"request_timeout_ms":1000,"attempt_timeout_ms":1000}
            },
            "consumed_draft":null
        },
        "accept_digest":digest, "expected_revisions":{"target":0},
        "idempotency_key":"budget-compatibility"
    });
    for (path, original) in [
        (vec!["operations", "find"], lookup),
        (
            vec!["operations", "find"],
            json!(["interactive_user", "change", "test", digest]),
        ),
        (vec!["routing", "apply"], publication),
    ] {
        for projection in [None, Some(false), Some(true)] {
            let mut request = original.clone();
            if let Some(projection) = projection {
                if let Some(fields) = request.as_array_mut() {
                    fields.push(json!(projection));
                } else {
                    request["include_model_budget_conflict"] = json!(projection);
                }
            }
            let bytes = serde_json::to_vec(&request).unwrap();
            std::fs::write(&file, &bytes).unwrap();
            for source in [
                vec!["--request-file", file.to_str().unwrap()],
                vec!["--request-stdin"],
            ] {
                let mut args = path.clone();
                args.extend(source);
                let (exit, envelope) = invoke(root.path(), &args, &bytes);
                // A syntactically valid request reaches transport; no daemon or grant is created.
                assert_eq!(exit, 6, "{args:?} {request}: {envelope}");
                assert_eq!(envelope["error"]["code"], "DAEMON_UNAVAILABLE");
            }
        }
    }
}

#[test]
fn existing_per_command_byte_limits_apply_to_files_and_stdin() {
    let root = tempfile::tempdir().unwrap();
    for (path, limit, request) in [
        (
            vec!["operations", "find"],
            65_536,
            json!({
                "principal_kind":"interactive_user","operation_kind":"change",
                "idempotency_key":"test","accepted_digest":format!("sha256:{}", "0".repeat(64))
            }),
        ),
        (
            vec!["worker", "dependencies", "select"],
            65_536,
            json!({
                "harness":"codex_cli","cli_path":"/tmp/hiroute-request-test/codex",
                "adapter_path":"/tmp/hiroute-request-test/codex-acp",
                "expected_selection_revision":0
            }),
        ),
        (vec!["routing", "options"], 1024 * 1024, json!({})),
    ] {
        // Whitespace is valid JSON framing, so oversize cannot hide behind syntax rejection.
        let mut bytes = serde_json::to_vec(&request).unwrap();
        for (size, expected_exit, expected_code) in [
            (limit, 6, "DAEMON_UNAVAILABLE"),
            (limit + 1, 2, "INVALID_ARGUMENTS"),
        ] {
            bytes.resize(size, b' ');
            let file = root.path().join("bounded.json");
            std::fs::write(&file, &bytes).unwrap();
            for source in [
                vec!["--request-file", file.to_str().unwrap()],
                vec!["--request-stdin"],
            ] {
                let mut args = path.clone();
                args.extend(source);
                args.extend(["--output", "json"]);
                let (exit, envelope) = invoke(root.path(), &args, &bytes);
                assert_eq!(exit, expected_exit, "{args:?}: {envelope}");
                assert_eq!(envelope["error"]["code"], expected_code);
            }
        }
    }
}

#[cfg(unix)]
#[test]
fn a_request_fifo_without_a_writer_is_rejected_without_opening_a_stream() {
    let root = tempfile::tempdir().unwrap();
    let fifo = root.path().join("request.pipe");
    nix::unistd::mkfifo(
        &fifo,
        nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
    )
    .unwrap();
    let (exit, envelope) = invoke(
        root.path(),
        &[
            "sessions",
            "receipt",
            "--request-file",
            fifo.to_str().unwrap(),
        ],
        b"",
    );
    assert_eq!(exit, 2, "{envelope}");
    assert_eq!(envelope["error"]["code"], "INVALID_ARGUMENTS");
}
