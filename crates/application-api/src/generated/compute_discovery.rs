use super::*;

pub(super) fn files() -> Vec<(&'static str, String)> {
    vec![
        (
            "compute-connection-options-request.v1.schema.json",
            pretty_json(
                &crate::command_schemas("compute.connection.options")
                    .expect("current options wire contract")
                    .request,
            ),
        ),
        (
            "compute-scan-request.union.schema.json",
            pretty_json(&json!({
                "$schema":"https://json-schema.org/draft/2020-12/schema",
                "$id":"hiroute://contracts/cli/compute-scan-request.union.schema.json",
                "oneOf":[{"type":"object","additionalProperties":false}, {
                    "type":"object","additionalProperties":false,"required":["schema"],
                    "properties":{
                        "schema":{"const":crate::COMPUTE_SCAN_REQUEST_SCHEMA_V2},
                        "native_target":{"$ref":"agent-scan-request.v1.schema.json#/properties/native_target"}
                    }
                }]
            })),
        ),
        (
            "prepare-discovered-model-connection-request.v1.schema.json",
            pretty_json(&json!({
                "$schema":"https://json-schema.org/draft/2020-12/schema",
                "$id":"hiroute://contracts/cli/prepare-discovered-model-connection-request.v1.schema.json",
                "type":"object","additionalProperties":false,"required":["discovery","prepare_id"],
                "properties":{
                    "prepare_id":{"type":"string","minLength":1,"maxLength":256},
                    "discovery":{"type":"object","additionalProperties":false,
                        "required":["discovery_ref","discovery_revision"],"properties":{
                            "discovery_ref":{"type":"string","pattern":"^discovery/[0-9a-f]{64}$"},
                            "discovery_revision":{"type":"integer","minimum":1,"maximum":18446744073709551615u64}
                        }
                    }
                }
            })),
        ),
    ]
}
