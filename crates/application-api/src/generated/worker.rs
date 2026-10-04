use serde_json::{Value, json};

fn reference() -> Value {
    json!({
        "type": "string",
        "minLength": 1,
        "maxLength": 256,
        "pattern": "^[A-Za-z0-9_./:-]+$"
    })
}

fn task_input() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["goal"],
        "description": "The final labeled prompt is limited to 256 KiB.",
        "properties": {
            "goal": {"type": "string", "minLength": 1, "maxLength": 262144},
            "context": {"type": "string", "maxLength": 262144},
            "constraints": {"type": "string", "maxLength": 262144},
            "acceptance_criteria": {"type": "string", "maxLength": 262144}
        }
    })
}

fn permission_policy() -> Value {
    json!({
        "enum": ["approve_all", "approve_reads", "deny_all"],
        "default": "approve_all"
    })
}

fn execution_properties() -> Value {
    json!({
        "permission_policy": permission_policy(),
        "run_timeout_secs": {"type": "integer", "minimum": 1, "maximum": 86400}
    })
}

pub(super) fn plans() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "hiroute://contracts/cli/worker-plans-request.v1.schema.json",
        "type": "object",
        "additionalProperties": false,
        "properties": {}
    })
}

pub(super) fn list() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "hiroute://contracts/cli/worker-list-request.v1.schema.json",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "title": {"type": "string", "minLength": 1, "maxLength": 4096},
            "cursor": {
                "type": "string",
                "minLength": 1,
                "maxLength": 4096
            },
            "limit": {"type": "integer", "minimum": 1, "maximum": 200}
        }
    })
}

pub(super) fn exec() -> Value {
    let mut properties = execution_properties()
        .as_object()
        .expect("static execution properties")
        .clone();
    properties.extend([
        (
            "schema".into(),
            json!({"const": "hiroute.worker-exec-request/v1"}),
        ),
        ("plan_id".into(), reference()),
        (
            "cwd".into(),
            json!({"type": "string", "minLength": 1, "maxLength": 4096}),
        ),
        ("input".into(), task_input()),
        ("submission_key".into(), reference()),
        (
            "title".into(),
            json!({"type": "string", "minLength": 1, "maxLength": 4096}),
        ),
        ("parent_task_ref".into(), reference()),
    ]);
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "hiroute://contracts/cli/worker-exec-request.v1.schema.json",
        "type": "object",
        "additionalProperties": false,
        "required": [
            "schema", "plan_id", "cwd", "run_timeout_secs", "input",
            "submission_key"
        ],
        "properties": properties
    })
}

pub(super) fn status() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "hiroute://contracts/cli/worker-status-request.v1.schema.json",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "task_id": reference(),
            "run_id": reference(),
            "submission_key": reference(),
            "submission_operation": {"enum": ["start", "continue"]}
        },
        "oneOf": [
            {
                "required": ["task_id"],
                "not": {"anyOf": [
                    {"required": ["submission_key"]},
                    {"required": ["submission_operation"]}
                ]}
            },
            {
                "required": ["run_id"],
                "not": {"anyOf": [
                    {"required": ["task_id"]},
                    {"required": ["submission_key"]},
                    {"required": ["submission_operation"]}
                ]}
            },
            {
                "required": ["submission_key", "submission_operation"],
                "not": {"anyOf": [
                    {"required": ["task_id"]},
                    {"required": ["run_id"]}
                ]}
            }
        ]
    })
}

pub(super) fn wait() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "hiroute://contracts/cli/worker-wait-request.v1.schema.json",
        "type": "object",
        "additionalProperties": false,
        "required": ["run_id", "wait_timeout_secs"],
        "properties": {
            "run_id": reference(),
            "after_revision": {"type": "integer", "minimum": 0},
            "wait_timeout_secs": {"type": "integer", "minimum": 1, "maximum": 30}
        }
    })
}

pub(super) fn result() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "hiroute://contracts/cli/worker-result-request.v1.schema.json",
        "type": "object",
        "additionalProperties": false,
        "required": ["run_id"],
        "properties": {
            "run_id": reference(),
            "offset": {"type": "integer", "minimum": 0, "maximum": u32::MAX},
            "max_bytes": {"type": "integer", "minimum": 1, "maximum": 1048576}
        }
    })
}

pub(super) fn cancel() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "hiroute://contracts/cli/worker-cancel-request.v1.schema.json",
        "type": "object",
        "additionalProperties": false,
        "required": ["run_id", "idempotency_key"],
        "properties": {
            "run_id": reference(),
            "idempotency_key": reference(),
            "reason": reference()
        }
    })
}

pub(super) fn continue_request() -> Value {
    let mut properties = execution_properties()
        .as_object()
        .expect("static execution properties")
        .clone();
    properties.extend([
        (
            "schema".into(),
            json!({"const": "hiroute.worker-continue-request/v1"}),
        ),
        ("task_id".into(), reference()),
        ("expected_latest_run_id".into(), reference()),
        (
            "cwd".into(),
            json!({"type": "string", "minLength": 1, "maxLength": 4096}),
        ),
        ("input".into(), task_input()),
        ("submission_key".into(), reference()),
    ]);
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "hiroute://contracts/cli/worker-continue-request.v1.schema.json",
        "type": "object",
        "additionalProperties": false,
        "required": [
            "schema", "task_id", "expected_latest_run_id", "run_timeout_secs",
            "input", "submission_key"
        ],
        "properties": properties
    })
}

pub(super) fn read() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "hiroute://contracts/cli/worker-read-request.v1.schema.json",
        "type": "object",
        "additionalProperties": false,
        "required": ["run_id"],
        "properties": {
            "run_id": reference(),
            "cursor": {"type": "string", "minLength": 1, "maxLength": 4096},
            "max_bytes": {"type": "integer", "minimum": 1, "maximum": 32768}
        }
    })
}

pub(super) fn dependencies_discover() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "hiroute://contracts/cli/worker-dependencies-discover-request.v1.schema.json",
        "type": "object",
        "additionalProperties": false,
        "properties": {"harness": {"enum": ["codex_cli", "claude_code", "qoder_cli"]}}
    })
}

pub(super) fn dependencies_select() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "hiroute://contracts/cli/worker-dependencies-select-request.v1.schema.json",
        "type": "object",
        "additionalProperties": false,
        "required": ["harness", "cli_path", "expected_selection_revision"],
        "properties": {
            "harness": {"enum": ["codex_cli", "claude_code", "qoder_cli"]},
            "adapter_path": {"type": "string", "minLength": 1, "maxLength": 4096},
            "cli_path": {"type": "string", "minLength": 1, "maxLength": 4096},
            "node_path": {"type": "string", "minLength": 1, "maxLength": 4096},
            "expected_selection_revision": {"type": "integer", "minimum": 0, "maximum": u64::MAX}
        },
        "if": {"properties": {"harness": {"const": "qoder_cli"}}},
        "then": {"properties": {"adapter_path": false, "node_path": false}},
        "else": {"required": ["adapter_path"]}
    })
}
