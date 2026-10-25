//! Slow read-only discovery must not own the mutable product Session.
use super::*;
use crate::authority::HostAuthority;
use std::sync::Arc;

pub async fn agent_snapshot(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    native_targets: Option<Vec<hiroute_domain::AgentNativeTargetV1>>,
    context_id: Option<String>,
) -> Result<AgentSnapshot, DesktopFailure> {
    main_window(window)?;
    let epoch = state.2.load(Ordering::SeqCst);
    let (client, authority, native_connection) = {
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
    let agents = crate::session::read_agent_entries_for_context(
        &client,
        native_targets.unwrap_or_default(),
        context_id.as_deref(),
    )
    .await?;
    let plans = crate::session::read_plan_catalog(&client).await?;

    let guard = confirmation_session(state, epoch).await?;
    let session = guard.as_ref().ok_or("RESIDENT_UNAVAILABLE")?;
    if !Arc::ptr_eq(&session.authority, &authority) {
        return Err("RESIDENT_UNAVAILABLE".into());
    }
    // The final current-service read runs under the short Session lock, so a
    // same-Session mutation cannot interleave its result with our publication.
    // Epoch alone is insufficient: a settings/Plan save can retain the Session.
    let after: ClientServiceStatusV1 = crate::session::query(
        &session.client,
        "GetClientServiceStatus",
        &ClientEmptyRequestV1 {},
    )
    .await?;
    // Window expiry can advance the epoch without taking the Session mutex.
    // Recheck after the last await; no later suspension may publish stale facts.
    if state.2.load(Ordering::SeqCst) != epoch {
        return Err("CONFIRMATION_STALE".into());
    }
    if !window.is_active() {
        return Err("CLIENT_CONTEXT_INVALID".into());
    }
    let current_native = session.authority.native_connection();
    if before.revisions != after.revisions
        || before.active_publication != after.active_publication
        || before.recovery_ready != after.recovery_ready
        || before.mutation_available != after.mutation_available
        || native_connection != current_native
    {
        return Err("AGENT_SNAPSHOT_CHANGED".into());
    }
    Ok(AgentSnapshot {
        agents,
        plans,
        trusted_authority: !matches!(
            current_native,
            Some(
                crate::NativeConnectionStatus::RecoveryRequired
                    | crate::NativeConnectionStatus::RestartRequired
                    | crate::NativeConnectionStatus::NotOwned
            )
        ),
    })
}

#[cfg(all(test, unix))]
mod tests;
