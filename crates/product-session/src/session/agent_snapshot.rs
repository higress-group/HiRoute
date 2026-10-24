//! Immutable Agent discovery/status I/O shared by Session and command readers.
use super::*;

pub(crate) async fn read_agent_entries(
    client: &Client,
    targets: Vec<hiroute_domain::AgentNativeTargetV1>,
) -> Result<Vec<AgentEntry>, DesktopFailure> {
    if targets.len() > 128 || targets.iter().any(|target| target.validate().is_err()) {
        return Err("AGENT_INPUT_INVALID".into());
    }
    #[derive(Deserialize)]
    struct Scan {
        agents: Vec<AgentEntry>,
    }
    let mut scan: Scan = query(client, "ListAgents", &ClientEmptyRequestV1 {}).await?;
    let automatic = targets.is_empty();
    let targets = if automatic {
        match super::agent_discovery::default_hermes_profile() {
            super::agent_discovery::ProfileDiscovery::Found(target) => vec![target],
            super::agent_discovery::ProfileDiscovery::Missing => Vec::new(),
            super::agent_discovery::ProfileDiscovery::Unavailable { target, reason } => {
                super::agent_discovery::merge_entry(
                    &mut scan.agents,
                    super::agent_discovery::unavailable_hermes(target, reason),
                );
                Vec::new()
            }
        }
    } else {
        targets
    };
    for target in targets {
        let selected: Result<Scan, DesktopFailure> = query(
            client,
            "ScanAgents",
            &hiroute_application_api::AgentScanRequestV1 {
                native_target: Some(target.clone()),
            },
        )
        .await;
        let selected = match selected {
            Ok(selected) => selected,
            Err(_) => {
                super::agent_discovery::merge_entry(
                    &mut scan.agents,
                    super::agent_discovery::unreadable_hermes(target),
                );
                continue;
            }
        };
        for agent in selected.agents {
            super::agent_discovery::merge_entry(&mut scan.agents, agent);
        }
    }
    for agent in &mut scan.agents {
        if let Some(context_id) = &agent.context_id {
            match query(
                client,
                "GetAgentConnectionStatus",
                &AgentSettingsStatusRequestV2 {
                    effective: None,
                    include_skills: true,
                    schema_version: AGENT_SETTINGS_SCHEMA_V2,
                    context_id: context_id.clone(),
                    native_target: agent.native_target.clone(),
                },
            )
            .await
            {
                Ok(status) => agent.settings = Some(status),
                Err(_) => agent.status_error = Some("AGENT_STATUS_UNAVAILABLE".into()),
            }
        }
    }
    Ok(scan.agents)
}
