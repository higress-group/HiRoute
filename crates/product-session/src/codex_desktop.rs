//! Native launch reads only its exact connection. No caller-supplied paths or grant.
use crate::{
    failure::DesktopFailure,
    session::{AgentEntry, query},
};
use hiroute_application_api::{
    AGENT_SETTINGS_SCHEMA_V2, AgentModelSettingsStateV2, AgentModelSurfaceV2, AgentScanRequestV1,
    AgentSettingsStatusProjectionV2, AgentSettingsStatusRequestV2, AgentSettingsStatusV2,
    ClientEmptyRequestV1, ClientServiceStatusV1, CodexDesktopInstanceViewV1,
};
use hiroute_client_core::Client;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchTarget {
    pub paths: CodexDesktopInstanceViewV1,
    pub operation: String,
}

pub fn launch_target(
    mut agents: Vec<AgentEntry>,
    context: &str,
    trusted: bool,
) -> Result<LaunchTarget, DesktopFailure> {
    if context.is_empty() || context.len() > 256 {
        return Err("AGENT_INPUT_INVALID".into());
    }
    if !trusted || agents.len() != 1 {
        return Err("AGENT_STATUS_UNVERIFIED".into());
    }
    let agent = agents.pop().ok_or("AGENT_STATUS_UNVERIFIED")?;
    if agent.agent_id != "agent_codex_desktop"
        || agent.context_id.as_deref() != Some(context)
        || agent.status_error.is_some()
        || !agent
            .available_surfaces
            .contains(&AgentModelSurfaceV2::CodexDesktop)
    {
        return Err("CODEX_DESKTOP_NOT_INSTALLED".into());
    }
    let model = agent
        .settings
        .as_ref()
        .and_then(|settings| settings.model())
        .ok_or("AGENT_STATUS_UNVERIFIED")?;
    if model.context_id != context
        || model.state != AgentModelSettingsStateV2::Configured
        || model.operation_state.as_deref() != Some("succeeded")
        || model.current_selection.is_none()
    {
        return Err("AGENT_STATUS_UNVERIFIED".into());
    }
    Ok(LaunchTarget {
        paths: agent.codex_desktop.ok_or("AGENT_STATUS_UNVERIFIED")?,
        operation: model
            .operation_id
            .clone()
            .ok_or("AGENT_STATUS_UNVERIFIED")?,
    })
}

/// Local Control authenticates the OS user. This does not accept a Web console credential.
pub async fn read_launch_target(
    client: &Client,
    context: &str,
) -> Result<LaunchTarget, DesktopFailure> {
    if context.is_empty() || context.len() > 256 {
        return Err("AGENT_INPUT_INVALID".into());
    }
    let before: ClientServiceStatusV1 =
        query(client, "GetClientServiceStatus", &ClientEmptyRequestV1 {}).await?;
    let agents = read_launch_entries(client, context).await?;
    let after: ClientServiceStatusV1 =
        query(client, "GetClientServiceStatus", &ClientEmptyRequestV1 {}).await?;
    require_current_service(&before, &after)?;
    launch_target(agents, context, true)
}

/// Hosts add their own Session/window fence around this exact model-only read.
pub(crate) async fn read_launch_entries(
    client: &Client,
    context: &str,
) -> Result<Vec<AgentEntry>, DesktopFailure> {
    if context.is_empty() || context.len() > 256 {
        return Err("AGENT_INPUT_INVALID".into());
    }
    #[derive(serde::Deserialize)]
    struct Scan {
        agents: Vec<AgentEntry>,
    }
    let mut scan: Scan = query(
        client,
        "ScanAgents",
        &AgentScanRequestV1 {
            context_id: Some(context.to_owned()),
            native_target: None,
        },
    )
    .await?;
    if scan.agents.len() != 1 || scan.agents[0].context_id.as_deref() != Some(context) {
        return Err("AGENT_STATUS_UNVERIFIED".into());
    }
    // Launch needs model authority only: unrelated skill inventories and Plans stay deferred.
    let settings: AgentSettingsStatusV2 = query(
        client,
        "GetAgentConnectionStatus",
        &AgentSettingsStatusRequestV2 {
            projection: AgentSettingsStatusProjectionV2::Model,
            effective: None,
            include_skills: false,
            schema_version: AGENT_SETTINGS_SCHEMA_V2,
            context_id: context.to_owned(),
            native_target: None,
        },
    )
    .await?;
    scan.agents[0].settings = Some(settings);
    Ok(scan.agents)
}

pub(crate) fn require_current_service(
    before: &ClientServiceStatusV1,
    after: &ClientServiceStatusV1,
) -> Result<(), DesktopFailure> {
    if !after.recovery_ready
        || !after.mutation_available
        || before.revisions != after.revisions
        || before.active_publication != after.active_publication
        || before.recovery_ready != after.recovery_ready
        || before.mutation_available != after.mutation_available
    {
        return Err("AGENT_STATUS_UNVERIFIED".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
