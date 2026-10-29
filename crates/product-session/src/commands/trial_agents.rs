//! Discovery choices and the selected model's strict proof have separate read contracts.
use super::*;
use crate::authority::HostAuthority;

fn trusted(read: &read_context::ReadContext) -> bool {
    !matches!(
        read.authority.native_connection(),
        Some(
            crate::NativeConnectionStatus::RecoveryRequired
                | crate::NativeConnectionStatus::RestartRequired
                | crate::NativeConnectionStatus::NotOwned
        )
    )
}

pub async fn trial_agent_choices(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
) -> Result<TrialAgentChoices, DesktopFailure> {
    main_window(window)?;
    let read = read_context::ReadContext::capture(state, false).await?;
    let before = read.status().await?;
    let choices = crate::session::read_trial_agent_choices(&read.client).await?;
    let _guard = read.finish(state, &before).await?;
    if !window.is_active() {
        return Err("CLIENT_CONTEXT_INVALID".into());
    }
    Ok(TrialAgentChoices {
        choices,
        trusted_authority: trusted(&read),
    })
}

pub async fn trial_agent_model(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    context_id: String,
) -> Result<TrialAgentModel, DesktopFailure> {
    main_window(window)?;
    if context_id.is_empty() || context_id.len() > 256 {
        return Err("AGENT_INPUT_INVALID".into());
    }
    let read = read_context::ReadContext::capture(state, false).await?;
    let before = read.status().await?;
    let settings: AgentModelSettingsStatusV2 = crate::session::query(
        &read.client,
        "GetAgentConnectionStatus",
        &AgentSettingsStatusRequestV2 {
            projection: AgentSettingsStatusProjectionV2::Model,
            effective: None,
            include_skills: false,
            native_target: None,
            schema_version: AGENT_SETTINGS_SCHEMA_V2,
            context_id: context_id.clone(),
        },
    )
    .await?;
    if settings.context_id != context_id {
        return Err("AGENT_STATUS_UNAVAILABLE".into());
    }
    let plans = crate::session::read_plan_catalog(&read.client).await?;
    let _guard = read.finish(state, &before).await?;
    if !window.is_active() {
        return Err("CLIENT_CONTEXT_INVALID".into());
    }
    Ok(TrialAgentModel {
        settings,
        plans,
        trusted_authority: trusted(&read),
    })
}
