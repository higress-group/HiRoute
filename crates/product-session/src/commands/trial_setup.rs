//! One service-fenced read over the daemon's combined trial capture.
use super::*;

pub async fn trial_setup_snapshot(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
) -> Result<TrialSetupSnapshotV1, DesktopFailure> {
    main_window(window)?;
    let read = read_context::ReadContext::capture(state, false).await?;
    let before = read.status().await?;
    let snapshot: TrialSetupSnapshotV1 = crate::session::query(
        &read.client,
        GET_TRIAL_SETUP_SNAPSHOT_OPERATION_V1,
        &ClientEmptyRequestV1::default(),
    )
    .await?;
    let _guard = read.finish(state, &before).await?;
    if !window.is_active() {
        return Err("CLIENT_CONTEXT_INVALID".into());
    }
    if snapshot.schema != TRIAL_SETUP_SNAPSHOT_SCHEMA_V1 {
        return Err("RESPONSE_DATA_INVALID".into());
    }
    Ok(snapshot)
}
