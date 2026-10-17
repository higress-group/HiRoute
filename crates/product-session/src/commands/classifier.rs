use super::{ClientContext, CommandState, DesktopFailure, main_window};
use hiroute_application_api::{ApplyResultV1, ClassifierDecisionTestResultV1};
use hiroute_domain::ComplexityClassifierModeV1;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClassifierDecisionTestInput {
    classifier: ComplexityClassifierModeV1,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClassifierHeaderSecretSaveInput {
    secret_id: String,
    secret: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionServiceSaveInput {
    change: hiroute_domain::DecisionServiceChangeV1,
    secret: Option<String>,
}

pub async fn decision_services(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
) -> Result<crate::session::DecisionServiceList, DesktopFailure> {
    main_window(window)?;
    state
        .0
        .lock()
        .await
        .as_ref()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .decision_services()
        .await
}

pub async fn save_decision_service(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    input: DecisionServiceSaveInput,
) -> Result<ApplyResultV1, DesktopFailure> {
    main_window(window)?;
    state
        .0
        .lock()
        .await
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .save_decision_service(input.change, input.secret.map(zeroize::Zeroizing::new))
        .await
}

/// Explicitly invokes the production classifier path with the product-owned
/// synthetic first turn. No real conversation or business model call is made.
pub async fn test_classifier_decision(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    input: ClassifierDecisionTestInput,
) -> Result<ClassifierDecisionTestResultV1, DesktopFailure> {
    main_window(window)?;
    state
        .0
        .lock()
        .await
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .test_classifier_decision(input.classifier)
        .await
}

pub async fn save_classifier_header_secret(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    input: ClassifierHeaderSecretSaveInput,
) -> Result<ApplyResultV1, DesktopFailure> {
    main_window(window)?;
    state
        .0
        .lock()
        .await
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .save_classifier_header_secret(input.secret_id, zeroize::Zeroizing::new(input.secret))
        .await
}
