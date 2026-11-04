//! Native host read: exact model state, with the current Session and window fences.
use super::*;
use crate::{authority::HostAuthority, codex_desktop::LaunchTarget};
use std::sync::Arc;

/// This is an internal host reader, not a Web console command or launch effect.
pub async fn native_codex_desktop_launch_target(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    context: &str,
) -> Result<LaunchTarget, DesktopFailure> {
    main_window(window)?;
    if context.is_empty() || context.len() > 256 {
        return Err("AGENT_INPUT_INVALID".into());
    }
    let epoch = state.2.load(Ordering::SeqCst);
    let (client, authority, native) = {
        let guard = confirmation_session(state, epoch).await?;
        let session = guard.as_ref().ok_or("RESIDENT_UNAVAILABLE")?;
        (
            session.client.clone(),
            session.authority.clone(),
            session.authority.native_connection(),
        )
    };
    let before: ClientServiceStatusV1 =
        crate::session::query(&client, "GetClientServiceStatus", &ClientEmptyRequestV1 {}).await?;
    let agents = crate::codex_desktop::read_launch_entries(&client, context).await?;
    let guard = confirmation_session(state, epoch).await?;
    let session = guard.as_ref().ok_or("RESIDENT_UNAVAILABLE")?;
    if !Arc::ptr_eq(&session.authority, &authority) {
        return Err("RESIDENT_UNAVAILABLE".into());
    }
    // Same-Session mutations cannot interleave the final service fence.
    let after: ClientServiceStatusV1 = crate::session::query(
        &session.client,
        "GetClientServiceStatus",
        &ClientEmptyRequestV1 {},
    )
    .await?;
    if state.2.load(Ordering::SeqCst) != epoch {
        return Err("CONFIRMATION_STALE".into());
    }
    if !window.is_active() {
        return Err("CLIENT_CONTEXT_INVALID".into());
    }
    let current_native = session.authority.native_connection();
    if native != current_native {
        return Err("AGENT_STATUS_UNVERIFIED".into());
    }
    crate::codex_desktop::require_current_service(&before, &after)?;
    let trusted = !matches!(
        current_native,
        Some(
            crate::NativeConnectionStatus::RecoveryRequired
                | crate::NativeConnectionStatus::RestartRequired
                | crate::NativeConnectionStatus::NotOwned
        )
    );
    crate::codex_desktop::launch_target(agents, context, trusted)
}

#[cfg(test)]
mod tests;
