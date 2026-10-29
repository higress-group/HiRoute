use super::*;

pub(super) fn externalize_tool_schema(
    value: &mut Value,
    replay: &ReplayStore,
    pool: &mut Option<ReplayContentWriter>,
    remaining: &mut usize,
) -> Result<(), ReplayError> {
    // Ingress backs large data leaves, not protocol structure. Keep this envelope
    // even without an encoding annotation: hiding it would also hide malformed
    // schema nodes from the exact same adapter validation used for inline input.
    // This walker charges every retained Value, map key and identity string.
    externalize_native_leaves(value, replay, pool, remaining)
}

pub(super) fn peer_identity(item_type: Option<&str>, key: &str, value: &Value) -> bool {
    item_type == Some("agent_message")
        && matches!(key, "author" | "recipient" | "id")
        && value
            .as_str()
            .is_some_and(|text| !text.contains(MARKER_PREFIX))
}
