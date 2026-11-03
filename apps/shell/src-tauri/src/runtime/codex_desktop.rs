//! The Windows shell accepts only an exact local Agent context, never commands or paths.
use super::*;
use hiroute_product_session::failure::DesktopFailure;

#[tauri::command]
pub(super) async fn shell_launch_codex_desktop(
    window: WebviewWindow,
    state: tauri::State<'_, Arc<ShellState>>,
    context_id: String,
) -> Result<serde_json::Value, DesktopFailure> {
    authorize(&window, &state)?;
    let client = service::client()?;
    let skills =
        hiroute_desktop_host_effects::launch_configured_codex_instance(&client, &context_id)
            .await?;
    Ok(serde_json::json!({"preserved_skills":skills.preserved}))
}
