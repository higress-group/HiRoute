//! Discovery remediation uses the existing envelope; no native paths escape.
use hiroute_application_api::{
    AgentDiscoveryListV1, DiscoveredAgentV1, MachineEnvelopeV2, NextActionV1,
};
use serde_json::{Value, json};

pub(crate) fn response(
    agents: Vec<DiscoveredAgentV1>,
    request_id: String,
    diagnostic: bool,
) -> MachineEnvelopeV2<Value> {
    let next_actions = if diagnostic {
        agents.iter().filter_map(remediation).collect()
    } else {
        Vec::new()
    };
    let mut response = crate::succeeded(AgentDiscoveryListV1 { agents }, request_id);
    response.next_actions = next_actions;
    response
}

fn remediation(agent: &DiscoveredAgentV1) -> Option<NextActionV1> {
    if agent.agent_id != "agent_dsh_default" || agent.supported {
        return None;
    }
    let reason = match agent.configuration_state.as_str() {
        "not_found_in_scope" => "DSH_WEB_EXECUTABLE_NOT_FOUND_CHECK_DAEMON_PATH",
        "executable_not_runnable" => "DSH_WEB_EXECUTABLE_NOT_RUNNABLE_CHECK_INSTALLATION",
        "executable_probe_unavailable" => "DSH_WEB_EXECUTABLE_PATH_UNAVAILABLE_CHECK_ACCESS",
        "executable_probe_timed_out" => "DSH_WEB_EXECUTABLE_PROBE_TIMED_OUT_RETRY",
        _ => return None,
    };
    Some(NextActionV1 {
        command_id: "agents.scan".into(),
        input: agent
            .context_id
            .as_ref()
            .map_or_else(|| json!({}), |context| json!({"context_id":context})),
        reason_code: reason.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(state: &str) -> DiscoveredAgentV1 {
        serde_json::from_value(json!({
            "agent_id":"agent_dsh_default", "profile_id":"dsh-responses-v1",
            "context_id":"agent-context/dsh/test", "version":"", "supported":false,
            "configuration_state":state
        }))
        .unwrap()
    }

    #[test]
    fn dsh_scan_explains_missing_unrunnable_and_unavailable_paths_without_native_data() {
        for (state, reason) in [
            (
                "not_found_in_scope",
                "DSH_WEB_EXECUTABLE_NOT_FOUND_CHECK_DAEMON_PATH",
            ),
            (
                "executable_not_runnable",
                "DSH_WEB_EXECUTABLE_NOT_RUNNABLE_CHECK_INSTALLATION",
            ),
            (
                "executable_probe_unavailable",
                "DSH_WEB_EXECUTABLE_PATH_UNAVAILABLE_CHECK_ACCESS",
            ),
            (
                "executable_probe_timed_out",
                "DSH_WEB_EXECUTABLE_PROBE_TIMED_OUT_RETRY",
            ),
        ] {
            let result = response(vec![agent(state)], "scan".into(), true);
            assert!(result.error.is_none());
            assert_eq!(
                result.data.unwrap()["agents"][0]["configuration_state"],
                state
            );
            assert_eq!(result.next_actions.len(), 1);
            assert_eq!(result.next_actions[0].reason_code, reason);
            assert_eq!(result.next_actions[0].command_id, "agents.scan");
            assert_eq!(
                result.next_actions[0].input,
                json!({"context_id":"agent-context/dsh/test"})
            );
        }
    }

    #[test]
    fn successful_and_unrelated_inventory_entries_do_not_request_dsh_repairs() {
        let mut other = agent("not_found_in_scope");
        other.agent_id = "agent_opencode_default".into();
        let mut supported = agent("registered");
        supported.supported = true;
        assert!(
            response(vec![other, supported], "scan".into(), true)
                .next_actions
                .is_empty()
        );
        assert!(
            response(vec![agent("not_found_in_scope")], "list".into(), false)
                .next_actions
                .is_empty()
        );
    }
}
