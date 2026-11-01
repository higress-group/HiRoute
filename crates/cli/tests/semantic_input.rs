use serde_json::{Value, json};
use std::io::Write;
use std::process::{Command, Stdio};

fn invoke(path: &[&str], request: &Value, rich: bool) -> (i32, Value, String) {
    let root = tempfile::tempdir().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_hiroute"));
    command
        .args(path)
        .arg("--request-stdin")
        .args(["--output", "json"])
        .env("HIROUTE_RUNTIME_DIR", root.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if rich {
        command.arg("--error-details");
    }
    let mut child = command.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(request).unwrap())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    (
        output.status.code().unwrap(),
        serde_json::from_slice(&output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

fn plan() -> Value {
    // Exact supported-release editor bytes; public synthetic identities replace runtime IDs.
    let fixture: Value = serde_json::from_str(include_str!(
        "../../domain/src/routing/authoring/fixtures/released-plan-editor.v2.json"
    ))
    .unwrap();
    json!({"change":{"schema":"hiroute.plan-content-change/v2",
        "target":{"intent":"create","creation_key":"semantic-input"}, "editor":fixture["editor"]}})
}

#[test]
fn semantic_failures_name_closed_fields_keep_legacy_shape_and_make_no_request() {
    for (pointer, value, expected, category) in [
        (
            "/change/editor/smart/judgment/degree/simple_threshold_millis",
            json!(1001),
            "$.change.editor.smart.judgment.degree.simple_threshold_millis",
            "out_of_range",
        ),
        (
            "/change/editor/smart/judgment/competence/floor_millis",
            json!(1001),
            "$.change.editor.smart.judgment.competence.floor_millis",
            "out_of_range",
        ),
        (
            "/change/editor/delegation_enabled",
            json!(true),
            "$.change.editor.work",
            "semantic_requirement",
        ),
        (
            "/change/editor/work",
            json!({"harness":"codex_cli","protocol":"chat_completions"}),
            "$.change.editor.work.protocol",
            "unsupported_protocol",
        ),
    ] {
        let mut request = plan();
        let (parent, field) = pointer.rsplit_once('/').unwrap();
        request.pointer_mut(parent).unwrap()[field] = value;
        request["change"]["editor"]["display_name"] = json!("PRIVATE_INPUT_NOT_FOR_DIAGNOSTICS");
        let (exit, rich, stderr) = invoke(&["routing", "preview"], &request, true);
        assert_eq!(exit, 2, "{rich}");
        assert_eq!(rich["error"]["code"], "INVALID_ARGUMENTS");
        assert_eq!(rich["data"]["diagnostic"]["path"], expected);
        assert_eq!(rich["data"]["diagnostic"]["category"], category);
        assert!(
            !rich
                .to_string()
                .contains("PRIVATE_INPUT_NOT_FOR_DIAGNOSTICS")
        );
        assert!(!stderr.contains("PRIVATE_INPUT_NOT_FOR_DIAGNOSTICS"));
        let (exit, legacy, stderr) = invoke(&["routing", "preview"], &request, false);
        assert_eq!(exit, 2);
        assert_eq!(legacy["error"], rich["error"]);
        assert!(legacy.get("data").is_none());
        assert!(stderr.contains(expected));
    }
    for (threshold, floor) in [(0, 1000), (1000, 0)] {
        let mut request = plan();
        request["change"]["editor"]["smart"]["judgment"]["degree"]["simple_threshold_millis"] =
            json!(threshold);
        request["change"]["editor"]["smart"]["judgment"]["competence"]["floor_millis"] =
            json!(floor);
        let (exit, response, _) = invoke(&["routing", "preview"], &request, true);
        assert_eq!(exit, 6, "valid boundary must reach transport: {response}");
        assert_eq!(response["error"]["code"], "DAEMON_UNAVAILABLE");
    }
}

#[test]
fn additional_model_protocol_error_does_not_echo_context_or_plan_keys() {
    let request = json!({"spec":{"schema_version":{"major":2,"minor":0},
        "context_id":"agent-context/PRIVATE_CONTEXT", "model":{"intent":"configure","settings":{
            "mode":"qoder_additional","allowed_plan_ids":["plan/PRIVATE_PLAN"],
            "plan_protocols":{"plan/PRIVATE_PLAN":"chat_completions"}}}}});
    let (exit, response, stderr) = invoke(&["agents", "connect", "preview"], &request, true);
    assert_eq!(exit, 2, "{response}");
    assert_eq!(
        response["data"]["diagnostic"]["path"],
        "$.spec.model.settings.plan_protocols.*"
    );
    assert_eq!(
        response["data"]["diagnostic"]["category"],
        "unsupported_protocol"
    );
    assert!(!format!("{response}{stderr}").contains("PRIVATE_"));
}

#[test]
fn installed_routing_schemas_publish_the_effective_thousandth_bounds() {
    fn bounds(value: &Value, found: &mut usize) {
        match value {
            Value::Object(object) => {
                if let Some(properties) = object.get("properties").and_then(Value::as_object) {
                    for name in ["simple_threshold_millis", "floor_millis"] {
                        if let Some(field) = properties.get(name) {
                            // Historical observation fields have separate contracts; this is
                            // the installed publication request's effective policy.
                            assert_eq!(field["maximum"], 1000, "{name}: {field}");
                            assert_eq!(field["minimum"], 0, "{name}: {field}");
                            *found += 1;
                        }
                    }
                }
                for child in object.values() {
                    bounds(child, found);
                }
            }
            Value::Array(values) => {
                for child in values {
                    bounds(child, found);
                }
            }
            _ => {}
        }
    }
    let schema = hiroute_application_api::command_schemas("routing.preview").unwrap();
    let mut found = 0;
    bounds(&schema.request, &mut found);
    assert!(found >= 2);
}

#[test]
fn incomplete_drafts_remain_legal_while_publication_reports_missing_work() {
    let mut request = plan();
    request["change"]["editor"]["delegation_enabled"] = json!(true);
    let editor: hiroute_domain::PlanEditorStateV2 =
        serde_json::from_value(request["change"]["editor"].clone()).unwrap();
    assert!(editor.validate_draft().is_ok());
    assert!(editor.effective().is_err());
    let (exit, error, _) = invoke(&["routing", "preview"], &request, true);
    assert_eq!(exit, 2);
    assert_eq!(error["data"]["diagnostic"]["path"], "$.change.editor.work");
}
