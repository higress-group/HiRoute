//! A context-bound native launch; WebView cannot supply paths, environment or commands.
use super::*;

#[tauri::command]
pub async fn launch_codex_desktop(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    context_id: String,
) -> Result<serde_json::Value, DesktopFailure> {
    main_window(&window)?;
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (state, context_id);
        Err("CODEX_DESKTOP_HOST_REQUIRED".into())
    }
    #[cfg(target_os = "macos")]
    {
        let snapshot = super::agent_snapshot(
            window.clone(),
            state.clone(),
            Some(Vec::new()),
            Some(context_id.clone()),
        )
        .await?;
        let (paths, operation) = launch_target(snapshot, &context_id)?;
        let prepared = paths.clone();
        let skills = tauri::async_runtime::spawn_blocking(move || {
            hiroute_desktop_host_effects::prepare_codex_instance(
                std::path::Path::new(&prepared.codex_home),
                std::path::Path::new(&prepared.user_data),
                std::path::Path::new(&prepared.source_codex_home),
                std::path::Path::new(&prepared.shared_skills),
            )
            .map_err(|_| DesktopFailure::from("CODEX_DESKTOP_PREPARE_FAILED"))
        })
        .await
        .map_err(|_| DesktopFailure::from("CODEX_DESKTOP_PREPARE_FAILED"))??;
        let fresh =
            super::agent_snapshot(window, state, Some(Vec::new()), Some(context_id.clone()))
                .await?;
        let (current, current_operation) = launch_target(fresh, &context_id)?;
        if paths != current || operation != current_operation {
            return Err("AGENT_STATUS_UNVERIFIED".into());
        }
        tauri::async_runtime::spawn_blocking(move || {
            hiroute_desktop_host_effects::launch_codex_instance(
                std::path::Path::new(&paths.codex_home),
                std::path::Path::new(&paths.user_data),
            )
            .map_err(DesktopFailure::from)
        })
        .await
        .map_err(|_| DesktopFailure::from("CODEX_DESKTOP_LAUNCH_FAILED"))??;
        Ok(serde_json::json!({ "preserved_skills": skills.preserved }))
    }
}

#[cfg(target_os = "macos")]
fn launch_target(
    snapshot: AgentSnapshot,
    context: &str,
) -> Result<(CodexDesktopInstanceViewV1, String), DesktopFailure> {
    if !snapshot.trusted_authority || snapshot.agents.len() != 1 {
        return Err("AGENT_STATUS_UNVERIFIED".into());
    }
    let agent = snapshot
        .agents
        .into_iter()
        .next()
        .ok_or("AGENT_STATUS_UNVERIFIED")?;
    if agent.agent_id != "agent_codex_desktop"
        || agent.context_id.as_deref() != Some(context)
        || agent.status_error.is_some()
        || !agent
            .available_surfaces
            .contains(&hiroute_domain::AgentModelSurfaceV2::CodexDesktop)
    {
        return Err("CODEX_DESKTOP_NOT_INSTALLED".into());
    }
    let model = agent
        .settings
        .as_ref()
        .and_then(|settings| settings.model())
        .ok_or("AGENT_STATUS_UNVERIFIED")?;
    if model.state != AgentModelSettingsStateV2::Configured
        || model.operation_state.as_deref() != Some("succeeded")
        || model.current_selection.is_none()
    {
        return Err("AGENT_STATUS_UNVERIFIED".into());
    }
    Ok((
        agent.codex_desktop.ok_or("AGENT_STATUS_UNVERIFIED")?,
        model
            .operation_id
            .clone()
            .ok_or("AGENT_STATUS_UNVERIFIED")?,
    ))
}
