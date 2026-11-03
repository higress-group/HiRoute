//! Shared local CLI/Windows Shell effect; completed settings are never replayed.
use hiroute_client_core::Client;
use hiroute_product_session::{codex_desktop::read_launch_target, failure::DesktopFailure};

pub async fn launch_configured_codex_instance(
    client: &Client,
    context: &str,
) -> Result<super::SharedCodexSkills, DesktopFailure> {
    let target = read_launch_target(client, context).await?;
    let paths = target.paths.clone();
    let skills = tokio::task::spawn_blocking(move || {
        super::prepare_codex_instance(
            paths.codex_home.as_ref(),
            paths.user_data.as_ref(),
            paths.source_codex_home.as_ref(),
            paths.shared_skills.as_ref(),
        )
    })
    .await
    .map_err(|_| DesktopFailure::from("CODEX_DESKTOP_PREPARE_FAILED"))?
    .map_err(|_| DesktopFailure::from("CODEX_DESKTOP_PREPARE_FAILED"))?;
    if read_launch_target(client, context).await? != target {
        return Err("AGENT_STATUS_UNVERIFIED".into());
    }
    tokio::task::spawn_blocking(move || {
        super::launch_codex_instance(
            target.paths.codex_home.as_ref(),
            target.paths.user_data.as_ref(),
        )
    })
    .await
    .map_err(|_| DesktopFailure::from("CODEX_DESKTOP_LAUNCH_FAILED"))?
    .map_err(DesktopFailure::from)?;
    Ok(skills)
}
