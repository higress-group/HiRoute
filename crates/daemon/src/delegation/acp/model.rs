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
    ensure_select(
        connection,
        session_id,
        config_options,
        "model",
        expected_model,
        cancellation,
        deadline,
    )
    .await
    .map(|_| ())
}

pub(super) async fn ensure_mode(
    connection: &ConnectionTo<Agent>,
    session_id: &str,
    config_options: Option<&[SessionConfigOption]>,
    expected_mode: &str,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<Vec<SessionConfigOption>, DelegationErrorV1> {
    ensure_select(
        connection,
        session_id,
        config_options,
        "mode",
        expected_mode,
        cancellation,
        deadline,
    )
    .await
}

async fn ensure_select(
    connection: &ConnectionTo<Agent>,
    session_id: &str,
    config_options: Option<&[SessionConfigOption]>,
    id: &str,
    expected: &str,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<Vec<SessionConfigOption>, DelegationErrorV1> {
    let model = selector(config_options, id)?;
    if model.current_value.to_string() == expected {
        return Ok(config_options.unwrap_or_default().to_vec());
    }
    let available = match &model.options {
        SessionConfigSelectOptions::Ungrouped(options) => options
            .iter()
            .any(|option| option.value.to_string() == expected),
        SessionConfigSelectOptions::Grouped(groups) => groups.iter().any(|group| {
            group
                .options
                .iter()
                .any(|option| option.value.to_string() == expected)
        }),
        _ => false,
    };
    if !available {
        return Err(DelegationErrorV1::CapabilityUnavailable);
    }
    let response = super::phase(
        if id == "mode" {
            "set-mode"
        } else {
            "set-model"
        },
        cancellation,
        deadline,
        connection
            .send_request(SetSessionConfigOptionRequest::new(
                session_id.to_owned(),
                id.to_owned(),
                expected,
            ))
            .block_task(),
    )
    .await
    .map_err(|error| match error {
        DelegationErrorV1::ProtocolFailed => DelegationErrorV1::CapabilityUnavailable,
        error => error,
    })?;
    if selector(Some(&response.config_options), id)?
        .current_value
        .to_string()
        != expected
    {
        return Err(DelegationErrorV1::CapabilityUnavailable);
    }
    Ok(response.config_options)
}

fn selector<'a>(
    options: Option<&'a [SessionConfigOption]>,
    id: &str,
) -> Result<&'a SessionConfigSelect, DelegationErrorV1> {
    let mut models = options
        .unwrap_or_default()
        .iter()
        .filter(|option| option.id.to_string() == id);
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
