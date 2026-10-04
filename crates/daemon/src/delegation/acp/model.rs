//! The supported Codex and Claude adapters expose the exact base model as the `model`
//! config selector. Legacy `models.currentModelId` is not the current typed ACP contract
//! and may append an effort suffix. Never fuzzy-match aliases or trust launch settings
//! alone: a resumed native session or user model allowlist can override those settings.
use agent_client_protocol::schema::v1::{
    SessionConfigKind, SessionConfigOption, SessionConfigSelect, SessionConfigSelectOptions,
    SetSessionConfigOptionRequest,
};
use agent_client_protocol::{Agent, ConnectionTo};
use hiroute_domain::delegation::DelegationErrorV1;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

pub(super) async fn ensure_model(
    connection: &ConnectionTo<Agent>,
    session_id: &str,
    config_options: Option<&[SessionConfigOption]>,
    expected_model: &str,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<(), DelegationErrorV1> {
    let model = model_selector(config_options)?;
    if model.current_value.to_string() == expected_model {
        return Ok(());
    }
    let available = match &model.options {
        SessionConfigSelectOptions::Ungrouped(options) => options
            .iter()
            .any(|option| option.value.to_string() == expected_model),
        SessionConfigSelectOptions::Grouped(groups) => groups.iter().any(|group| {
            group
                .options
                .iter()
                .any(|option| option.value.to_string() == expected_model)
        }),
        _ => false,
    };
    if !available {
        return Err(DelegationErrorV1::CapabilityUnavailable);
    }
    let response = super::phase(
        cancellation,
        deadline,
        connection
            .send_request(SetSessionConfigOptionRequest::new(
                session_id.to_owned(),
                "model",
                expected_model,
            ))
            .block_task(),
    )
    .await
    .map_err(|error| match error {
        DelegationErrorV1::ProtocolFailed => DelegationErrorV1::CapabilityUnavailable,
        error => error,
    })?;
    if model_selector(Some(&response.config_options))?
        .current_value
        .to_string()
        != expected_model
    {
        return Err(DelegationErrorV1::CapabilityUnavailable);
    }
    Ok(())
}

fn model_selector(
    options: Option<&[SessionConfigOption]>,
) -> Result<&SessionConfigSelect, DelegationErrorV1> {
    let mut models = options
        .unwrap_or_default()
        .iter()
        .filter(|option| option.id.to_string() == "model");
    let Some(option) = models.next() else {
        return Err(DelegationErrorV1::CapabilityUnavailable);
    };
    if models.next().is_some() {
        return Err(DelegationErrorV1::CapabilityUnavailable);
    }
    match &option.kind {
        SessionConfigKind::Select(model) => Ok(model),
        _ => Err(DelegationErrorV1::CapabilityUnavailable),
    }
}
