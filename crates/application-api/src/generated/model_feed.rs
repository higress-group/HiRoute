//! Feed query schemas are derived from the versioned DTOs used by every host.
use super::*;
fn schema<T: schemars::JsonSchema>(id: &str) -> String {
    let mut value = serde_json::to_value(schemars::schema_for!(T)).expect("schema serializes");
    value["$id"] = json!(format!("hiroute://contracts/cli/{id}.schema.json"));
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
            schema::<crate::ModelFeedSettingsRequestV1>("model-feed-settings.v1"),
        ),
        (
            "model-feed-remove.v1.schema.json",
            schema::<crate::ModelFeedRemoveRequestV1>("model-feed-remove.v1"),
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
