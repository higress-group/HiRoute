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
        let target = read_launch_target(&window, &state, &context_id).await?;
        let (paths, operation) = (target.paths, target.operation);
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
        let fresh = read_launch_target(&window, &state, &context_id).await?;
        if paths != fresh.paths || operation != fresh.operation {
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
async fn read_launch_target(
    window: &WebviewWindow,
    state: &DesktopState,
    context: &str,
) -> Result<hiroute_product_session::codex_desktop::LaunchTarget, DesktopFailure> {
    let broker = window.state::<WebConfirmationState>();
    let active = || window.is_visible().is_ok();
    let visible = || window.is_visible().unwrap_or(false);
    let window_context = hiroute_product_session::commands::ClientContext {
        id: window.label(),
        confirmations: &broker,
        active: &active,
        visible: &visible,
        notify: None,
    };
    let product_state = hiroute_product_session::commands::CommandState(
        &state.0,
        &state.1.error,
        &state.2,
        &state.3,
    );
    hiroute_product_session::commands::native_codex_desktop_launch_target(
        &window_context,
        &product_state,
        context,
    )
    .await
}

#[cfg(all(test, target_os = "macos"))]
fn launch_target(
    snapshot: AgentSnapshot,
    context: &str,
) -> Result<(CodexDesktopInstanceViewV1, String), DesktopFailure> {
    let target = hiroute_product_session::codex_desktop::launch_target(
        snapshot.agents,
        context,
        snapshot.trusted_authority,
    )?;
    Ok((target.paths, target.operation))
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    fn snapshot(state: &str, agent_id: &str, trusted: bool) -> AgentSnapshot {
        AgentSnapshot {
            trusted_authority: trusted,
            plans: AgentPlanCatalogViewV2 { schema: "hiroute.agent-plan-catalog/v2".into(), next_cursor: None, drafts: vec![], plans: vec![] },
            agents: vec![serde_json::from_value(serde_json::json!({
                "agent_id": agent_id, "context_id": "context/desktop", "version": "fixture",
                "configuration_state": "registered", "available_surfaces": ["codex_desktop"],
                "codex_desktop": { "codex_home": "/private/instance/codex", "user_data": "/private/instance/desktop", "source_codex_home": "/ordinary/.codex", "shared_skills": "/ordinary/.agents/skills" },
                "settings": { "schema": "hiroute.agent-model-settings-status/v2", "context_id": "context/desktop",
                    "state": state, "operation_id": "operation/desktop", "operation_state": "succeeded",
                    "restore_point_ref": "restore/desktop", "applied_revision": 1, "surface_results": [], "model_verified": false,
                    "current_selection": { "mode": "codex_default", "native_model_mode": "hiroute_only", "fixed_models": [], "allowed_plan_ids": ["plan/one"], "default_selection": { "kind": "plan", "plan_id": "plan/one" } }
                }
            })).unwrap()],
        }
    }
    #[test]
    fn codex_desktop_launch_requires_current_authority_and_exact_connection() {
        assert!(
            launch_target(
                snapshot("configured", "agent_codex_desktop", true),
                "context/desktop"
            )
            .is_ok()
        );
        for state in ["pending", "not_configured", "drift", "needs_attention"] {
            assert!(
                launch_target(
                    snapshot(state, "agent_codex_desktop", true),
                    "context/desktop"
                )
                .is_err()
            );
        }
        assert!(
            launch_target(
                snapshot("configured", "agent_codex_desktop", false),
                "context/desktop"
            )
            .is_err()
        );
        assert!(
            launch_target(
                snapshot("configured", "agent_codex_default", true),
                "context/desktop"
            )
            .is_err()
        );
        assert!(
            launch_target(
                snapshot("configured", "agent_codex_desktop", true),
                "context/other"
            )
            .is_err()
        );
    }
}
