use super::{DesktopFailure, DesktopState, main_window};
use hiroute_application_api::{ApplyResultV1, ClassifierDecisionTestResultV1};
use hiroute_domain::ComplexityClassifierModeV1;
use tauri::{State, WebviewWindow};
use tauri_plugin_dialog::DialogExt;

const CLASSIFIER_OPENAPI_FILENAME: &str = "hiroute-decision.openapi.json";
const CLASSIFIER_OPENAPI: &str =
    include_str!("../../../../../decision-extensions/api/decision.openapi.json");

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ClassifierDecisionTestInput {
    classifier: ComplexityClassifierModeV1,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ClassifierHeaderSecretSaveInput {
    secret_id: String,
    secret: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ClassifierOpenApiSaveOutcome {
    Saved,
    Cancelled,
}

fn write_classifier_openapi(path: &std::path::Path) -> std::io::Result<()> {
    std::fs::write(path, CLASSIFIER_OPENAPI.as_bytes())
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DecisionServiceSaveInput {
    change: hiroute_domain::DecisionServiceChangeV1,
    secret: Option<String>,
}

#[tauri::command]
pub(super) async fn decision_services(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<crate::session::DecisionServiceList, DesktopFailure> {
    main_window(&window)?;
    state
        .0
        .lock()
        .await
        .as_ref()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .decision_services()
        .await
}

#[tauri::command]
pub(super) async fn save_decision_service(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: DecisionServiceSaveInput,
) -> Result<ApplyResultV1, DesktopFailure> {
    main_window(&window)?;
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
#[tauri::command]
pub(super) async fn test_classifier_decision(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: ClassifierDecisionTestInput,
) -> Result<ClassifierDecisionTestResultV1, DesktopFailure> {
    main_window(&window)?;
    state
        .0
        .lock()
        .await
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .test_classifier_decision(input.classifier)
        .await
}

#[tauri::command]
pub(super) async fn save_classifier_header_secret(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: ClassifierHeaderSecretSaveInput,
) -> Result<ApplyResultV1, DesktopFailure> {
    main_window(&window)?;
    state
        .0
        .lock()
        .await
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .save_classifier_header_secret(input.secret_id, zeroize::Zeroizing::new(input.secret))
        .await
}

#[tauri::command]
pub(super) async fn save_classifier_openapi(
    window: WebviewWindow,
) -> Result<ClassifierOpenApiSaveOutcome, DesktopFailure> {
    main_window(&window)?;
    let selected = window
        .dialog()
        .file()
        .set_parent(&window)
        .set_file_name(CLASSIFIER_OPENAPI_FILENAME)
        .add_filter("OpenAPI JSON", &["json"])
        .blocking_save_file();
    let Some(selected) = selected else {
        return Ok(ClassifierOpenApiSaveOutcome::Cancelled);
    };
    let path = selected
        .into_path()
        .map_err(|_| DesktopFailure::from("CLASSIFIER_OPENAPI_SAVE_UNAVAILABLE"))?;
    write_classifier_openapi(&path)
        .map_err(|_| DesktopFailure::from("CLASSIFIER_OPENAPI_SAVE_UNAVAILABLE"))?;
    Ok(ClassifierOpenApiSaveOutcome::Saved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_writer_emits_the_canonical_contract() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(CLASSIFIER_OPENAPI_FILENAME);
        write_classifier_openapi(&path).unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), CLASSIFIER_OPENAPI);
        let document: serde_json::Value = serde_json::from_str(CLASSIFIER_OPENAPI).unwrap();
        assert_eq!(document["openapi"], "3.1.0");
    }
}
