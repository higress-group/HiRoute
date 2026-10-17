//! Independent sign-in management. Callback material never enters ordinary Local Control JSON.
use crate::HostAuthority;

use super::{ClientContext, CommandState, main_window};
use crate::failure::DesktopFailure;
use hiroute_application_api::*;

fn login_data(
    envelope: MachineEnvelopeV2<ComputeSubscriptionLoginResultV1>,
) -> Result<ComputeSubscriptionLoginResultV1, DesktopFailure> {
    // The login backend projects only closed reason codes. Do not forward raw OAuth errors or
    // serialize an unexpected envelope, which could accidentally expose an authorization URL.
    if envelope.error.is_some() {
        return Err("SUBSCRIPTION_LOGIN_UNAVAILABLE".into());
    }
    envelope.data.ok_or_else(|| "RESPONSE_DATA_MISSING".into())
}

pub async fn manage_subscription_login(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    request: ComputeSubscriptionLoginRequestV1,
) -> Result<ComputeSubscriptionLoginResultV1, DesktopFailure> {
    main_window(window)?;
    if !request.valid() {
        return Err("REQUEST_INVALID".into());
    }
    let client = state
        .0
        .lock()
        .await
        .as_ref()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .client
        .clone();
    login_data(
        client
            .manage_subscription_login(&crate::random_id()?, request)
            .await?,
    )
}

pub async fn submit_subscription_login_callback(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    login_ref: String,
    callback: String,
) -> Result<ComputeSubscriptionLoginResultV1, DesktopFailure> {
    let callback = zeroize::Zeroizing::new(callback);
    main_window(window)?;
    let client = state
        .0
        .lock()
        .await
        .as_ref()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .client
        .clone();
    let status = login_data(
        client
            .manage_subscription_login(
                &crate::random_id()?,
                ComputeSubscriptionLoginRequestV1::Status {
                    login_ref: login_ref.clone(),
                },
            )
            .await?,
    )?;
    let session = status
        .sessions
        .into_iter()
        .find(|session| session.login_ref == login_ref)
        .ok_or("SUBSCRIPTION_LOGIN_STALE")?;
    if session.status != SubscriptionLoginStatusV1::Pending {
        return Err("SUBSCRIPTION_LOGIN_STALE".into());
    }
    let input_candidate = session
        .callback_input_candidate
        .ok_or("SUBSCRIPTION_LOGIN_STALE")?;
    let callback_request_id = crate::random_id()?;
    state
        .0
        .lock()
        .await
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .authority
        .register_subscription_callback_input(&input_candidate, callback)?;

    let response = client
        .manage_subscription_login(
            &callback_request_id,
            ComputeSubscriptionLoginRequestV1::Callback {
                login_ref,
                input_candidate: input_candidate.clone(),
            },
        )
        .await;
    // Release even after an uncertain response: a consumed input is already gone, and an
    // unconsumed callback must not remain available to an unrelated later operation.
    if let Some(session) = state.0.lock().await.as_mut() {
        session.authority.release_model_input(&input_candidate)?;
    }
    login_data(response?)
}
