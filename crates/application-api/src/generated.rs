use std::collections::BTreeMap;
mod delegation;
mod plan_authoring;
mod subscription_login;
mod worker;

use hiroute_domain::{
    CanonicalDigest, PRODUCT_CONTRACT_REVISION, PROPOSAL_MAP_REVISION, SchemaVersion,
};
use serde::Serialize;
use serde_json::{Value, json};

use crate::commands::{descriptor_digest, planned_manifest, release_manifest};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeneratedContractFile {
    pub relative_path: &'static str,
    pub contents: String,
}

fn pretty<T: Serialize>(value: &T) -> String {
    let mut rendered =
        serde_json::to_string_pretty(value).expect("static contract data is serializable");
    rendered.push('\n');
    rendered
}

// ACP's transitive serde_json/preserve_order feature must not change published schema bytes.
fn pretty_json(value: &Value) -> String {
    pretty(&hiroute_domain::canonicalize_json(value.clone()))
}

fn coverage_manifest() -> Value {
    let commands = planned_manifest().commands;
    json!({
        "schema_version": { "major": 1, "minor": 0 },
        "descriptor_digest": descriptor_digest(),
        "commands": commands.into_iter().map(|command| json!({
            "command_id": command.command_id,
            "lifecycle": command.lifecycle,
            "positive": command.positive,
            "negative": command.negative,
        })).collect::<Vec<_>>(),
    })
}

fn machine_envelope_v2_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "hiroute://contracts/cli/machine-envelope.v2.schema.json",
        "title": "HiRoute machine envelope v2",
        "type": "object",
        "additionalProperties": false,
        "required": ["schema_version", "status", "warnings", "next_actions"],
        "properties": {
            "schema_version": { "$ref": "#/$defs/schema_version" },
            "request_id": { "type": "string", "minLength": 1 },
            "status": {
                "enum": [
                    "succeeded", "accepted", "usage_error", "conflict", "denied",
                    "not_found", "unavailable", "action_required", "needs_attention",
                    "internal_error"
                ]
            },
            "data": true,
            "operation": { "$ref": "#/$defs/operation" },
            "warnings": {
                "type": "array",
                "items": { "$ref": "#/$defs/warning" }
            },
            "next_actions": {
                "type": "array",
                "items": { "$ref": "#/$defs/next_action" }
            },
            "error": { "$ref": "#/$defs/error" }
        },
        "$defs": {
            "schema_version": {
                "type": "object",
                "additionalProperties": false,
                "required": ["major", "minor"],
                "properties": {
                    "major": { "const": 2 },
                    "minor": { "type": "integer", "minimum": 0 }
                }
            },
            "warning": {
                "type": "object",
                "additionalProperties": false,
                "required": ["code", "details_schema"],
                "properties": {
                    "code": { "type": "string", "minLength": 1 },
                    "details_schema": { "type": "string", "minLength": 1 }
                }
            },
            "next_action": {
                "type": "object",
                "additionalProperties": false,
                "required": ["command_id", "input", "reason_code"],
                "properties": {
                    "command_id": { "type": "string", "minLength": 1 },
                    "input": { "type": "object" },
                    "reason_code": { "type": "string", "pattern": "^[A-Z][A-Z0-9_]+$" }
                }
            },
            "operation": {
                "type": "object", "additionalProperties": false,
                "required": ["operation_id", "state", "sequence", "cancellable"],
                "properties": {
                    "operation_id": { "type": "string", "minLength": 1 },
                    "state": { "type": "string", "minLength": 1 },
                    "sequence": { "type": "integer", "minimum": 0 },
                    "cancellable": { "type": "boolean" }
                }
            },
            "error": {
                "type": "object",
                "additionalProperties": false,
                "required": ["code", "category", "message_key", "retryable", "details_schema"],
                "properties": {
                    "code": { "type": "string", "pattern": "^[A-Z][A-Z0-9_]+$" },
                    "category": { "enum": ["usage", "conflict", "authorization", "not_found", "unavailable", "action_required", "recovery", "internal"] },
                    "message_key": { "type": "string", "minLength": 1 },
                    "retryable": { "type": "boolean" },
                    "operation_id": { "type": "string", "minLength": 1 },
                    "details_schema": { "type": "string", "minLength": 1 }
                }
            }
        }
    })
}

fn local_control_v2_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "hiroute://contracts/cli/local-control.v2.schema.json",
        "title": "HiRoute Local Control request v2",
        "type": "object",
        "additionalProperties": false,
        "required": ["schema_version", "request_id", "operation_id", "payload"],
        "properties": {
            "schema_version": {
                "type": "object",
                "additionalProperties": false,
                "required": ["major", "minor"],
                "properties": {
                    "major": { "const": 2 },
                    "minor": { "type": "integer", "minimum": 0 }
                }
            },
            "request_id": { "type": "string", "minLength": 1 },
            "operation_id": { "type": "string", "minLength": 1 },
            "payload": true,
            "protected_grant": {
                "type": "object", "additionalProperties": false,
                "required": ["principal_kind", "capability"],
                "properties": {
                    "principal_kind": { "enum": ["interactive_user", "desktop", "skill", "sealed_collaboration"] },
                    "capability": {
                        "type": "string", "minLength": 1, "writeOnly": true,
                        "description": "Protected launcher/IPC value; never argv, plain environment, output, or persistence."
                    }
                },
                "if": {"properties":{"principal_kind":{"const":"sealed_collaboration"}}},
                "then": {"properties":{"capability":{"maxLength":4096}}},
                "else": {"properties":{"capability":{"maxLength":512}}}
            }
        }
    })
}

fn client_hello_v2_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "hiroute://contracts/cli/local-control-hello.v2.schema.json",
        "type": "object",
        "additionalProperties": false,
        "required": ["api_version", "machine_schema_version", "client_name", "client_version"],
        "properties": {
            "api_version": { "$ref": "local-control.v2.schema.json#/properties/schema_version" },
            "machine_schema_version": { "$ref": "local-control.v2.schema.json#/properties/schema_version" },
            "client_name": { "type": "string", "minLength": 1, "maxLength": 128 },
            "client_version": { "const": crate::LOCAL_CONTROL_RELEASE_VERSION }
        }
    })
}

fn setup_request_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "hiroute://contracts/cli/setup-request.v1.schema.json",
        "type": "object", "additionalProperties": false, "required": ["schema"],
        "properties": {
            "schema": { "const": "hiroute.setup-request/v1" },
            "agent_ids": { "type": "array", "uniqueItems": true, "items": { "type": "string", "minLength": 1, "maxLength": 256 } },
            "routing_mode": { "enum": ["smart_saving", "free_first", "custom"] },
            "routing_purpose": { "type": "string", "minLength": 1, "maxLength": 512 },
            "free_pool_mode": { "enum": ["automatic_all_available", "manual"] },
            "fallback_policy": { "enum": ["free_only", "primary_fallback"] },
            "native_subagent_routing": { "enum": ["automatic", "enabled", "disabled"] },
            "codex_catalog": { "enum": ["automatic", "enabled", "disabled"] }
        }
    })
}

fn setup_apply_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "hiroute://contracts/cli/setup-apply-request.v1.schema.json",
        "type": "object", "additionalProperties": false,
        "required": ["spec", "accept_digest", "expected_revision", "idempotency_key"],
        "properties": {
            "spec": { "$ref": "setup-request.v1.schema.json" },
            "accept_digest": { "type": "string", "pattern": "^sha256:[0-9a-f]{64}$" },
            "expected_revision": { "type": "integer", "minimum": 0 },
            "idempotency_key": { "type": "string", "minLength": 1, "maxLength": 256 }
        }
    })
}

fn lookup_schema(id: &str, title: &str) -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema", "$id": id,
        "title": title, "type": "object", "additionalProperties": false,
        "required": ["id"],
        "properties": {
            "id": { "type": "string", "minLength": 1 },
            "content": { "enum": ["none", "messages", "messages-and-tools"] }
        }
    })
}

fn operation_lookup_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "hiroute://contracts/cli/operation-lookup.v1.schema.json",
        "type": "object", "additionalProperties": false, "required": ["operation_id"],
        "properties": {
            "operation_id": { "type": "string", "pattern": "^op_[0-9a-f]{32}$" },
            "after_sequence": { "type": "integer", "minimum": 0 }
        }
    })
}

fn operation_cancel_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "hiroute://contracts/cli/operation-cancel-request.v1.schema.json",
        "type": "object", "additionalProperties": false,
        "required": ["operation_id", "idempotency_key"],
        "properties": {
            "operation_id": { "type": "string", "pattern": "^op_[0-9a-f]{32}$" },
            "idempotency_key": { "type": "string", "minLength": 1, "maxLength": 256 }
        }
    })
}

fn agent_check_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "hiroute://contracts/cli/agent-check-request.v1.schema.json",
        "type": "object", "additionalProperties": false,
        "required": ["agent_id", "scope", "suite", "allow_model_call"],
        "properties": {
            "agent_id": { "type": "string", "minLength": 1 },
            "scope": { "enum": ["configuration", "native_authentication", "collaboration", "live"] },
            "suite": { "enum": ["quick", "tool", "conformance"] },
            "allow_model_call": { "type": "boolean" }
        }
    })
}

fn agent_launch_descriptor_request_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "hiroute://contracts/cli/agent-launch-descriptor-request.v1.schema.json",
        "type": "object", "additionalProperties": false,
        "required": ["connection_id"],
        "properties": {
            "connection_id": {
                "type": "string", "minLength": 1, "maxLength": 256,
                "pattern": "^agent-connection/[A-Za-z0-9._/-]+$"
            }
        }
    })
}

fn empty_request_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "hiroute://contracts/cli/empty-request.v1.schema.json",
        "type": "object", "additionalProperties": false
    })
}

fn session_list_query_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "hiroute://contracts/cli/session-list-query.v1.schema.json",
        "type": "object", "additionalProperties": false,
        "properties": {
            "from_ms": { "type": "integer" }, "to_ms": { "type": "integer" },
            "agent_id": { "type": "string", "minLength": 1 },
            "query": { "type": "string", "minLength": 1 },
            "model_switch": { "enum": ["any", "only", "exclude"] },
            "include_unlinked": { "type": "boolean" },
            "limit": { "type": "integer", "minimum": 1, "maximum": 200 },
            "cursor": { "type": "string", "minLength": 1, "maxLength": 512 }
        }
    })
}

fn value_query_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "hiroute://contracts/cli/value-query.v1.schema.json",
        "type": "object", "additionalProperties": false,
        "properties": {
            "agent_plan_id": { "type": "string", "minLength": 1 },
            "from_ms": { "type": "integer" }, "to_ms": { "type": "integer" },
            "period": { "enum": ["today", "seven_days", "thirty_days"] },
            "group_by": { "enum": ["none", "day"] },
            "currency": { "type": "string", "minLength": 1 },
            "session_id": { "type": "string", "minLength": 1 }
        }
    })
}

pub fn generated_contract_files() -> Vec<GeneratedContractFile> {
    let mut files = BTreeMap::from([
        (
            "subscription-login-request.v1.schema.json",
            pretty_json(&subscription_login::request()),
        ),
        (
            "subscription-login-result.v1.schema.json",
            pretty_json(&subscription_login::response()),
        ),
        (
            "delegation-cancel-request.v1.schema.json",
            pretty_json(&delegation::cancel()),
        ),
        (
            "delegation-continue-request.v1.schema.json",
            pretty_json(&delegation::continue_request()),
        ),
        (
            "delegation-get-request.v1.schema.json",
            pretty_json(&delegation::get()),
        ),
        (
            "delegation-list-request.v1.schema.json",
            pretty_json(&delegation::list()),
        ),
        (
            "delegation-start-request.v1.schema.json",
            pretty_json(&delegation::start()),
        ),
        (
            "delegation-result-request.v1.schema.json",
            pretty_json(&delegation::result()),
        ),
        (
            "delegation-wait-request.v1.schema.json",
            pretty_json(&delegation::wait()),
        ),
        (
            "work-plan-list-request.v1.schema.json",
            pretty_json(&json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "$id": "hiroute://contracts/cli/work-plan-list-request.v1.schema.json",
                "type": "object", "additionalProperties": false,
                "required": ["workspace_id", "context_id", "grant_id"],
                "properties": {
                    "workspace_id": {"type": "string", "minLength": 1},
                    "context_id": {"type": "string", "minLength": 1, "maxLength": 256},
                    "grant_id": {"type": "string", "minLength": 1, "maxLength": 256}
                }
            })),
        ),
        (
            "worker-residual-confirm-request.v1.schema.json",
            pretty_json(&worker::residual_confirm()),
        ),
        (
            "worker-cancel-request.v1.schema.json",
            pretty_json(&worker::cancel()),
        ),
        (
            "worker-continue-request.v1.schema.json",
            pretty_json(&worker::continue_request()),
        ),
        (
            "worker-exec-request.v1.schema.json",
            pretty_json(&worker::exec()),
        ),
        (
            "worker-dependencies-discover-request.v1.schema.json",
            pretty_json(&worker::dependencies_discover()),
        ),
        (
            "worker-dependencies-select-request.v1.schema.json",
            pretty_json(&worker::dependencies_select()),
        ),
        (
            "worker-list-request.v1.schema.json",
            pretty_json(&worker::list()),
        ),
        (
            "worker-result-request.v1.schema.json",
            pretty_json(&worker::result()),
        ),
        (
            "worker-read-request.v1.schema.json",
            pretty_json(&worker::read()),
        ),
        (
            "worker-plans-request.v1.schema.json",
            pretty_json(&worker::plans()),
        ),
        (
            "worker-status-request.v1.schema.json",
            pretty_json(&worker::status()),
        ),
        (
            "worker-wait-request.v1.schema.json",
            pretty_json(&worker::wait()),
        ),
        (
            "agent-launch-descriptor-request.v1.schema.json",
            pretty_json(&agent_launch_descriptor_request_schema()),
        ),
        (
            "empty-request.v1.schema.json",
            pretty_json(&empty_request_schema()),
        ),
        (
            "coverage-manifest.v1.json",
            pretty_json(&coverage_manifest()),
        ),
        (
            "local-control.v2.schema.json",
            pretty_json(&local_control_v2_schema()),
        ),
        (
            "local-control-hello.v2.schema.json",
            pretty_json(&client_hello_v2_schema()),
        ),
        (
            "machine-envelope.v2.schema.json",
            pretty_json(&machine_envelope_v2_schema()),
        ),
        ("manifest.v1.json", pretty(&release_manifest())),
        (
            "operation-cancel-request.v1.schema.json",
            pretty_json(&operation_cancel_schema()),
        ),
        (
            "operation-lookup.v1.schema.json",
            pretty_json(&operation_lookup_schema()),
        ),
        ("planned-manifest.v1.json", pretty(&planned_manifest())),
        (
            "agent-check-request.v1.schema.json",
            pretty_json(&agent_check_schema()),
        ),
        (
            "session-lookup.v1.schema.json",
            pretty_json(&lookup_schema(
                "hiroute://contracts/cli/session-lookup.v1.schema.json",
                "HiRoute session or receipt lookup v1",
            )),
        ),
        (
            "session-list-query.v1.schema.json",
            pretty_json(&session_list_query_schema()),
        ),
        (
            "setup-apply-request.v1.schema.json",
            pretty_json(&setup_apply_schema()),
        ),
        (
            "setup-request.v1.schema.json",
            pretty_json(&setup_request_schema()),
        ),
        (
            "value-query.v1.schema.json",
            pretty_json(&value_query_schema()),
        ),
    ]);

    files.extend(plan_authoring::files());
    let file_digests = files
        .iter()
        .map(|(path, contents)| {
            (
                (*path).to_owned(),
                CanonicalDigest::of_bytes(contents.as_bytes()),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let contract_set = json!({
        "schema_version": SchemaVersion::new(1, 0),
        "product_contract_revision": PRODUCT_CONTRACT_REVISION,
        "proposal_map_revision": PROPOSAL_MAP_REVISION,
        "descriptor_digest": descriptor_digest(),
        "files": file_digests,
    });
    files.insert("contract-set.v1.json", pretty_json(&contract_set));

    files
        .into_iter()
        .map(|(relative_path, contents)| GeneratedContractFile {
            relative_path,
            contents,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_contract_set_is_complete_and_stably_ordered() {
        let files = generated_contract_files();
        let paths = files
            .iter()
            .map(|file| file.relative_path)
            .collect::<Vec<_>>();
        assert_eq!(
            paths,
            vec![
                "agent-check-request.v1.schema.json",
                "agent-launch-descriptor-request.v1.schema.json",
                "agent-plan-catalog-query.v2.schema.json",
                "contract-set.v1.json",
                "coverage-manifest.v1.json",
                "delegation-cancel-request.v1.schema.json",
                "delegation-continue-request.v1.schema.json",
                "delegation-get-request.v1.schema.json",
                "delegation-list-request.v1.schema.json",
                "delegation-result-request.v1.schema.json",
                "delegation-start-request.v1.schema.json",
                "delegation-wait-request.v1.schema.json",
                "empty-request.v1.schema.json",
                "local-control-hello.v2.schema.json",
                "local-control.v2.schema.json",
                "machine-envelope.v2.schema.json",
                "manifest.v1.json",
                "operation-cancel-request.v1.schema.json",
                "operation-lookup.v1.schema.json",
                "plan-content-change.v2.schema.json",
                "plan-draft-change.v1.schema.json",
                "plan-editor.v2.schema.json",
                "plan-lifecycle-change.v1.schema.json",
                "planned-manifest.v1.json",
                "session-list-query.v1.schema.json",
                "session-lookup.v1.schema.json",
                "setup-apply-request.v1.schema.json",
                "setup-request.v1.schema.json",
                "subscription-login-request.v1.schema.json",
                "subscription-login-result.v1.schema.json",
                "value-query.v1.schema.json",
                "work-plan-list-request.v1.schema.json",
                "worker-cancel-request.v1.schema.json",
                "worker-continue-request.v1.schema.json",
                "worker-dependencies-discover-request.v1.schema.json",
                "worker-dependencies-select-request.v1.schema.json",
                "worker-exec-request.v1.schema.json",
                "worker-list-request.v1.schema.json",
                "worker-plans-request.v1.schema.json",
                "worker-read-request.v1.schema.json",
                "worker-residual-confirm-request.v1.schema.json",
                "worker-result-request.v1.schema.json",
                "worker-status-request.v1.schema.json",
                "worker-wait-request.v1.schema.json",
            ]
        );
        assert!(files.iter().all(|file| file.contents.ends_with('\n')));
    }
}
