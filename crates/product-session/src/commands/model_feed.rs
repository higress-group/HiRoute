//! Shared daemon-owned feed and trial reads; the client cannot supply a URL or verification key.
use super::*;

async fn query<T: serde::de::DeserializeOwned, I: serde::Serialize>(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    operation: &str,
    input: &I,
) -> Result<T, DesktopFailure> {
    main_window(window)?;
    let (client, authority) = {
        let guard = state.0.lock().await;
        let session = guard.as_ref().ok_or("RESIDENT_UNAVAILABLE")?;
        (session.client.clone(), session.authority.clone())
    };
    let result = crate::session::query(&client, operation, input).await?;
    let guard = state.0.lock().await;
    if !guard
        .as_ref()
        .is_some_and(|session| std::sync::Arc::ptr_eq(&session.authority, &authority))
    {
        return Err("RESIDENT_UNAVAILABLE".into());
    }
    Ok(result)
}

pub async fn model_feed_status(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
) -> Result<ModelFeedStatusV2, DesktopFailure> {
    query(
        window,
        state,
        "GetModelFeedStatus",
        &ClientEmptyRequestV1::default(),
    )
    .await
}

pub async fn model_feed_refresh(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
) -> Result<ModelFeedStatusV2, DesktopFailure> {
    query(
        window,
        state,
        "RefreshModelFeed",
        &ClientEmptyRequestV1::default(),
    )
    .await
}

pub async fn model_feed_settings_apply(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    input: ModelFeedSettingsRequestV1,
) -> Result<ModelFeedStatusV2, DesktopFailure> {
    query(window, state, "ApplyModelFeedSettings", &input).await
}

pub async fn model_feed_remove_preview(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
) -> Result<ModelFeedRemovalPreviewV2, DesktopFailure> {
    query(
        window,
        state,
        "PreviewModelFeedRemoval",
        &ClientEmptyRequestV1::default(),
    )
    .await
}

pub async fn model_feed_remove_apply(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    input: ModelFeedRemoveRequestV1,
) -> Result<ModelFeedStatusV2, DesktopFailure> {
    query(window, state, "ApplyModelFeedRemoval", &input).await
}

pub async fn model_availability(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
) -> Result<ModelAvailabilityResultV2, DesktopFailure> {
    query(
        window,
        state,
        "GetModelAvailability",
        &ClientEmptyRequestV1::default(),
    )
    .await
}

pub async fn preview_agent_previous_routing(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    input: AgentPreviousRoutingQueryV1,
) -> Result<AgentPreviousRoutingPreviewV1, DesktopFailure> {
    query(window, state, "PreviewAgentPreviousRouting", &input).await
}

pub async fn list_model_metadata(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    input: ListModelMetadataV1,
) -> Result<ModelMetadataPageV1, DesktopFailure> {
    query(window, state, "ListModelMetadata", &input).await
}
pub async fn get_model_metadata(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    input: GetModelMetadataV1,
) -> Result<ModelMetadataDetailV1, DesktopFailure> {
    query(window, state, "GetModelMetadata", &input).await
}

#[cfg(all(test, unix))]
mod tests;
