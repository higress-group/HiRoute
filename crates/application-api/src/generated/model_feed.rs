//! Feed query schemas are derived from the versioned DTOs used by every host.
use super::*;
fn schema<T: schemars::JsonSchema>(id: &str) -> String {
    let mut value = serde_json::to_value(schemars::schema_for!(T)).expect("schema serializes");
    value["$id"] = json!(format!("hiroute://contracts/cli/{id}.schema.json"));
    pretty_json(&value)
}
fn retained_request(name: &str, properties: Value, required: &[&str], minimum: bool) -> String {
    let mut value = json!({"$schema":"https://json-schema.org/draft/2020-12/schema",
        "$id":format!("hiroute://contracts/cli/{name}.v1.schema.json"),"type":"object",
        "additionalProperties":false,"properties":properties,"required":required});
    if minimum {
        value["minProperties"] = json!(1);
    }
    pretty_json(&value)
}
pub(super) fn files() -> Vec<(&'static str, String)> {
    vec![
        (
            "model-feed-status.v2.schema.json",
            schema::<crate::ModelFeedStatusV2>("model-feed-status.v2"),
        ),
        (
            "model-feed-settings.v1.schema.json",
            retained_request(
                "model-feed-settings",
                json!({"retrieval_enabled":{"type":["boolean","null"]},"acknowledge_disclosure":{"type":["boolean","null"]}}),
                &[],
                true,
            ),
        ),
        (
            "model-feed-remove.v1.schema.json",
            retained_request(
                "model-feed-remove",
                json!({"accepted_digest":{"type":"string","pattern":"^sha256:[0-9a-f]{64}$"},"also_disable_retrieval":{"type":"boolean"}}),
                &["accepted_digest", "also_disable_retrieval"],
                false,
            ),
        ),
        (
            "model-feed-removal-preview.v2.schema.json",
            schema::<crate::ModelFeedRemovalPreviewV2>("model-feed-removal-preview.v2"),
        ),
        (
            "model-availability.v2.schema.json",
            schema::<crate::ModelAvailabilityResultV2>("model-availability.v2"),
        ),
    ]
}
