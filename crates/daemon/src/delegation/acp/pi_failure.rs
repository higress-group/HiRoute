//! Only the Pi bridge's closed failure vocabulary is allowed through this boundary.
//! Native messages/data are never copied into task results or diagnostics.
use hiroute_domain::delegation::DelegationErrorV1;

pub(super) fn map_error(error: agent_client_protocol::Error) -> DelegationErrorV1 {
    let fallback = DelegationErrorV1::ProtocolFailed;
    if error.code != (-32000).into() {
        return fallback;
    }
    let Some(data) = error.data.as_ref().and_then(serde_json::Value::as_object) else {
        return fallback;
    };
    if data.len() != 2
        || data.get("schema").and_then(serde_json::Value::as_str)
            != Some("hiroute.pi-worker-failure/v1")
    {
        return fallback;
    }
    match data.get("stage").and_then(serde_json::Value::as_str) {
        Some("sdk_load") => DelegationErrorV1::DependenciesInvalid,
        Some(
            "configuration" | "sdk_capability" | "route_binding" | "resources" | "session_create",
        ) => DelegationErrorV1::CapabilityUnavailable,
        Some("history" | "session_load") => DelegationErrorV1::ResumeUnavailable,
        Some("prompt") => DelegationErrorV1::PromptFailed,
        Some("request") => DelegationErrorV1::InvalidArguments,
        Some("busy") => DelegationErrorV1::Conflict,
        _ => fallback,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unrelated_or_open_ended_adapter_errors_are_not_trusted() {
        for data in [
            json!({"schema":"foreign", "stage":"prompt"}),
            json!({"schema":"hiroute.pi-worker-failure/v1", "stage":"future"}),
            json!({"schema":"hiroute.pi-worker-failure/v1", "stage":"prompt", "secret":"never copy"}),
        ] {
            assert_eq!(
                map_error(agent_client_protocol::Error::new(-32000, "secret").data(data)),
                DelegationErrorV1::ProtocolFailed
            );
        }
    }
}
