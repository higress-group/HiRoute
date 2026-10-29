//! Immutable Agent discovery/status I/O shared by Session and command readers.
use super::*;

pub(crate) async fn read_agent_entries(
    client: &Client,
    targets: Vec<hiroute_domain::AgentNativeTargetV1>,
) -> Result<Vec<AgentEntry>, DesktopFailure> {
    read_agent_entries_for_context(client, targets, None).await
}

pub(crate) async fn read_agent_entries_for_context(
    client: &Client,
    targets: Vec<hiroute_domain::AgentNativeTargetV1>,
    context_id: Option<&str>,
) -> Result<Vec<AgentEntry>, DesktopFailure> {
    let mut agents = if let Some(context) = context_id {
        if context.is_empty()
            || context.len() > 256
            || targets.len() > 1
            || targets.iter().any(|target| target.validate().is_err())
        {
            return Err("AGENT_INPUT_INVALID".into());
        }
        #[derive(Deserialize)]
        struct Scan {
            agents: Vec<AgentEntry>,
        }
        let scan: Scan = query(
            client,
            "ScanAgents",
            &hiroute_application_api::AgentScanRequestV1 {
                context_id: Some(context.to_owned()),
                native_target: targets.into_iter().next(),
            },
        )
        .await?;
        if scan.agents.len() != 1 || scan.agents[0].context_id.as_deref() != Some(context) {
            return Err("AGENT_STATUS_UNAVAILABLE".into());
        }
        scan.agents
    } else {
        discover_agent_entries(client, targets).await?
    };
    for agent in &mut agents {
        read_settings(client, agent).await;
    }
    Ok(agents)
}

pub(crate) async fn read_trial_agent_choices(
    client: &Client,
) -> Result<Vec<TrialAgentChoice>, DesktopFailure> {
    let agents = discover_agent_entries(client, Vec::new()).await?;
    Ok(agents
        .into_iter()
        .map(|agent| TrialAgentChoice {
            agent_id: agent.agent_id,
            context_id: agent.context_id,
            native_target: agent.native_target,
            version: agent.version,
            configuration_state: agent.configuration_state,
            available_surfaces: agent.available_surfaces,
            native_model_catalog: agent.native_model_catalog,
            codex_access: agent.codex_access,
        })
        .collect())
}

async fn discover_agent_entries(
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
                context_id: None,
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
    Ok(scan.agents)
}

async fn read_settings(client: &Client, agent: &mut AgentEntry) {
    if let Some(context_id) = &agent.context_id {
        match query(
            client,
            "GetAgentConnectionStatus",
            &AgentSettingsStatusRequestV2 {
                projection: Default::default(),
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
