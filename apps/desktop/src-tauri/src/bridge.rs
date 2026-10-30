use hiroute_application_api::*;
use hiroute_product_session::commands::{
    ClassifierDecisionTestInput, ClassifierHeaderSecretSaveInput, DecisionServiceSaveInput,
    DecisionServiceSaveInputV2, ObservationDeleteInput, ProtectedModelInputRegistration,
    WebComputeCandidateViewV2, WebComputeConnectionApplyRequestV1, WebComputeDiscoveryRefV1,
    WebComputeManagementChangeV2, WebComputeSavePreviewV2, WebComputeSaveResultV2,
    WebComputeScanResultV2, WebComputeSubscriptionCandidatesV2,
    WebComputeSubscriptionCheckResultV2, WebValidationRefV2, WorkerDependencyConfirmationInput,
    WorkerDependencyConfirmationState, WorkerDependencyConfirmationV1,
};
use tauri::Emitter;
// WebView supplies explicit intent and renders prompts; native preview and typed policy own submission.
use crate::failure::DesktopFailure;
use crate::session::{
    AgentCheckCompletion, AgentCheckInput, AgentOutcome, AgentSettingsInput, AgentSnapshot,
    DesktopSnapshot, EditorInput, ModelSaveAccepted, MutationOutcome, PriceDisplayResult,
    PriceEditInput, RenameInput, RestoreInput, Session,
};
use hiroute_application_api::{
    ClientOperationViewV1, GetEffectivePricesV2, ModelCatalogQueryV1, ModelCatalogResultV1,
};
use hiroute_diagnostics::context::DiagnosticHandle;
use serde_json::Value;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use tauri::{Manager, State, WebviewWindow};
use tokio::sync::Mutex;
mod previous_routing;
use previous_routing::*;
mod trial_agents;
use trial_agents::*;
mod agent_home;
use agent_home::*;
mod model_feed;
use model_feed::*;
mod native_network;
use native_network::*;
mod model_connections;
use model_connections::*;
mod subscription_login;
use subscription_login::*;
mod observation;
use observation::*;
mod classifier;
use classifier::*;
mod worker_tasks;
use worker_tasks::*;
mod diagnostics;
use diagnostics::*;
mod external_url;
use external_url::*;
mod host_settings;
mod subscription_proxy;
use host_settings::*;
use subscription_proxy::*;
mod connection_recovery;
mod startup;
use connection_recovery::*;
pub use diagnostics::DiagnosticsState;
pub use startup::StartupState;
use startup::*;
mod web_confirmation;
pub(crate) use web_confirmation::expire_window_confirmations;
use web_confirmation::*;
mod window_behavior;
use window_behavior::*;
mod updates;
use updates::*;
mod launch_presentation;
mod shutdown;
use launch_presentation::LaunchPresentation;
pub struct DesktopState(
    pub Arc<Mutex<Option<Session>>>,
    pub Arc<StartupState>,
    pub AtomicU64,
    /// Process diagnostics for the protected Desktop action path; independent of the business Session.
    pub DiagnosticHandle,
);

fn desktop_data_root(handle: &tauri::AppHandle) -> Result<std::path::PathBuf, ()> {
    let root = handle.path().app_local_data_dir().map_err(|_| ())?;
    #[cfg(debug_assertions)]
    let root = std::env::var_os("HIROUTE_DESKTOP_TEST_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or(root);
    Ok(root)
}

fn main_window(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err("WINDOW_DENIED".into())
    }
}

#[tauri::command]
pub async fn desktop_snapshot(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<DesktopSnapshot, DesktopFailure> {
    main_window(&window)?;
    let broker = window.state::<WebConfirmationState>();
    let active = || window.is_visible().is_ok();
    let visible = || window.is_visible().unwrap_or(false);
    let notify = |request: &WebConfirmationRequest| {
        window
            .emit_to(
                tauri::EventTarget::webview(window.label()),
                "hiroute-web-confirmation",
                request,
            )
            .map_err(|_| "CONFIRMATION_UNAVAILABLE".to_owned())
    };
    let context = hiroute_product_session::commands::ClientContext {
        id: window.label(),
        confirmations: &broker,
        active: &active,
        visible: &visible,
        notify: Some(&notify),
    };
    let product_state = hiroute_product_session::commands::CommandState(
        &state.0,
        &state.1.error,
        &state.2,
        &state.3,
    );
    hiroute_product_session::commands::desktop_snapshot(&context, &product_state).await
}
#[tauri::command]
pub async fn refresh_route_references(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    language: String,
) -> Result<MutationOutcome, DesktopFailure> {
    main_window(&window)?;
    let broker = window.state::<WebConfirmationState>();
    let active = || window.is_visible().is_ok();
    let visible = || window.is_visible().unwrap_or(false);
    let notify = |request: &WebConfirmationRequest| {
        window
            .emit_to(
                tauri::EventTarget::webview(window.label()),
                "hiroute-web-confirmation",
                request,
            )
            .map_err(|_| "CONFIRMATION_UNAVAILABLE".to_owned())
    };
    let context = hiroute_product_session::commands::ClientContext {
        id: window.label(),
        confirmations: &broker,
        active: &active,
        visible: &visible,
        notify: Some(&notify),
    };
    let product_state = hiroute_product_session::commands::CommandState(
        &state.0,
        &state.1.error,
        &state.2,
        &state.3,
    );
    hiroute_product_session::commands::refresh_route_references(&context, &product_state, language)
        .await
}
#[tauri::command]
pub async fn preview_plan_editor(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: EditorInput,
) -> Result<MutationOutcome, DesktopFailure> {
    main_window(&window)?;
    let broker = window.state::<WebConfirmationState>();
    let active = || window.is_visible().is_ok();
    let visible = || window.is_visible().unwrap_or(false);
    let notify = |request: &WebConfirmationRequest| {
        window
            .emit_to(
                tauri::EventTarget::webview(window.label()),
                "hiroute-web-confirmation",
                request,
            )
            .map_err(|_| "CONFIRMATION_UNAVAILABLE".to_owned())
    };
    let context = hiroute_product_session::commands::ClientContext {
        id: window.label(),
        confirmations: &broker,
        active: &active,
        visible: &visible,
        notify: Some(&notify),
    };
    let product_state = hiroute_product_session::commands::CommandState(
        &state.0,
        &state.1.error,
        &state.2,
        &state.3,
    );
    hiroute_product_session::commands::preview_plan_editor(&context, &product_state, input).await
}
#[tauri::command]
pub async fn plan_editor_options(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: hiroute_application_api::PlanEditorOptionsRequestV1,
) -> Result<hiroute_application_api::PlanEditorOptionsV1, DesktopFailure> {
    main_window(&window)?;
    let broker = window.state::<WebConfirmationState>();
    let active = || window.is_visible().is_ok();
    let visible = || window.is_visible().unwrap_or(false);
    let notify = |request: &WebConfirmationRequest| {
        window
            .emit_to(
                tauri::EventTarget::webview(window.label()),
                "hiroute-web-confirmation",
                request,
            )
            .map_err(|_| "CONFIRMATION_UNAVAILABLE".to_owned())
    };
    let context = hiroute_product_session::commands::ClientContext {
        id: window.label(),
        confirmations: &broker,
        active: &active,
        visible: &visible,
        notify: Some(&notify),
    };
    let product_state = hiroute_product_session::commands::CommandState(
        &state.0,
        &state.1.error,
        &state.2,
        &state.3,
    );
    hiroute_product_session::commands::plan_editor_options(&context, &product_state, input).await
}
#[tauri::command]
pub async fn preview_rename(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: RenameInput,
) -> Result<MutationOutcome, DesktopFailure> {
    main_window(&window)?;
    let broker = window.state::<WebConfirmationState>();
    let active = || window.is_visible().is_ok();
    let visible = || window.is_visible().unwrap_or(false);
    let notify = |request: &WebConfirmationRequest| {
        window
            .emit_to(
                tauri::EventTarget::webview(window.label()),
                "hiroute-web-confirmation",
                request,
            )
            .map_err(|_| "CONFIRMATION_UNAVAILABLE".to_owned())
    };
    let context = hiroute_product_session::commands::ClientContext {
        id: window.label(),
        confirmations: &broker,
        active: &active,
        visible: &visible,
        notify: Some(&notify),
    };
    let product_state = hiroute_product_session::commands::CommandState(
        &state.0,
        &state.1.error,
        &state.2,
        &state.3,
    );
    hiroute_product_session::commands::preview_rename(&context, &product_state, input).await
}
#[tauri::command]
pub async fn preview_restore_name(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: RestoreInput,
) -> Result<MutationOutcome, DesktopFailure> {
    main_window(&window)?;
    let broker = window.state::<WebConfirmationState>();
    let active = || window.is_visible().is_ok();
    let visible = || window.is_visible().unwrap_or(false);
    let notify = |request: &WebConfirmationRequest| {
        window
            .emit_to(
                tauri::EventTarget::webview(window.label()),
                "hiroute-web-confirmation",
                request,
            )
            .map_err(|_| "CONFIRMATION_UNAVAILABLE".to_owned())
    };
    let context = hiroute_product_session::commands::ClientContext {
        id: window.label(),
        confirmations: &broker,
        active: &active,
        visible: &visible,
        notify: Some(&notify),
    };
    let product_state = hiroute_product_session::commands::CommandState(
        &state.0,
        &state.1.error,
        &state.2,
        &state.3,
    );
    hiroute_product_session::commands::preview_restore_name(&context, &product_state, input).await
}
#[tauri::command]
pub async fn preview_price_change(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: PriceEditInput,
) -> Result<MutationOutcome, DesktopFailure> {
    main_window(&window)?;
    let broker = window.state::<WebConfirmationState>();
    let active = || window.is_visible().is_ok();
    let visible = || window.is_visible().unwrap_or(false);
    let notify = |request: &WebConfirmationRequest| {
        window
            .emit_to(
                tauri::EventTarget::webview(window.label()),
                "hiroute-web-confirmation",
                request,
            )
            .map_err(|_| "CONFIRMATION_UNAVAILABLE".to_owned())
    };
    let context = hiroute_product_session::commands::ClientContext {
        id: window.label(),
        confirmations: &broker,
        active: &active,
        visible: &visible,
        notify: Some(&notify),
    };
    let product_state = hiroute_product_session::commands::CommandState(
        &state.0,
        &state.1.error,
        &state.2,
        &state.3,
    );
    hiroute_product_session::commands::preview_price_change(&context, &product_state, input).await
}
#[tauri::command]
pub async fn model_reference_query(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: ModelCatalogQueryV1,
) -> Result<ModelCatalogResultV1, DesktopFailure> {
    main_window(&window)?;
    let broker = window.state::<WebConfirmationState>();
    let active = || window.is_visible().is_ok();
    let visible = || window.is_visible().unwrap_or(false);
    let notify = |request: &WebConfirmationRequest| {
        window
            .emit_to(
                tauri::EventTarget::webview(window.label()),
                "hiroute-web-confirmation",
                request,
            )
            .map_err(|_| "CONFIRMATION_UNAVAILABLE".to_owned())
    };
    let context = hiroute_product_session::commands::ClientContext {
        id: window.label(),
        confirmations: &broker,
        active: &active,
        visible: &visible,
        notify: Some(&notify),
    };
    let product_state = hiroute_product_session::commands::CommandState(
        &state.0,
        &state.1.error,
        &state.2,
        &state.3,
    );
    hiroute_product_session::commands::model_reference_query(&context, &product_state, input).await
}
#[tauri::command]
pub async fn effective_price_query(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: GetEffectivePricesV2,
) -> Result<PriceDisplayResult, DesktopFailure> {
    main_window(&window)?;
    let broker = window.state::<WebConfirmationState>();
    let active = || window.is_visible().is_ok();
    let visible = || window.is_visible().unwrap_or(false);
    let notify = |request: &WebConfirmationRequest| {
        window
            .emit_to(
                tauri::EventTarget::webview(window.label()),
                "hiroute-web-confirmation",
                request,
            )
            .map_err(|_| "CONFIRMATION_UNAVAILABLE".to_owned())
    };
    let context = hiroute_product_session::commands::ClientContext {
        id: window.label(),
        confirmations: &broker,
        active: &active,
        visible: &visible,
        notify: Some(&notify),
    };
    let product_state = hiroute_product_session::commands::CommandState(
        &state.0,
        &state.1.error,
        &state.2,
        &state.3,
    );
    hiroute_product_session::commands::effective_price_query(&context, &product_state, input).await
}
#[tauri::command]
pub async fn agent_snapshot(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    native_targets: Option<Vec<hiroute_domain::AgentNativeTargetV1>>,
    context_id: Option<String>,
) -> Result<AgentSnapshot, DesktopFailure> {
    main_window(&window)?;
    let broker = window.state::<WebConfirmationState>();
    let active = || window.is_visible().is_ok();
    let visible = || window.is_visible().unwrap_or(false);
    let notify = |request: &WebConfirmationRequest| {
        window
            .emit_to(
                tauri::EventTarget::webview(window.label()),
                "hiroute-web-confirmation",
                request,
            )
            .map_err(|_| "CONFIRMATION_UNAVAILABLE".to_owned())
    };
    let context = hiroute_product_session::commands::ClientContext {
        id: window.label(),
        confirmations: &broker,
        active: &active,
        visible: &visible,
        notify: Some(&notify),
    };
    let product_state = hiroute_product_session::commands::CommandState(
        &state.0,
        &state.1.error,
        &state.2,
        &state.3,
    );
    hiroute_product_session::commands::agent_snapshot(
        &context,
        &product_state,
        native_targets,
        context_id,
    )
    .await
}
#[tauri::command]
pub async fn retry_agent_settings(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: hiroute_application_api::AgentSettingsRetryV1,
) -> Result<Value, DesktopFailure> {
    main_window(&window)?;
    let broker = window.state::<WebConfirmationState>();
    let active = || window.is_visible().is_ok();
    let visible = || window.is_visible().unwrap_or(false);
    let notify = |request: &WebConfirmationRequest| {
        window
            .emit_to(
                tauri::EventTarget::webview(window.label()),
                "hiroute-web-confirmation",
                request,
            )
            .map_err(|_| "CONFIRMATION_UNAVAILABLE".to_owned())
    };
    let context = hiroute_product_session::commands::ClientContext {
        id: window.label(),
        confirmations: &broker,
        active: &active,
        visible: &visible,
        notify: Some(&notify),
    };
    let product_state = hiroute_product_session::commands::CommandState(
        &state.0,
        &state.1.error,
        &state.2,
        &state.3,
    );
    hiroute_product_session::commands::retry_agent_settings(&context, &product_state, input).await
}
#[tauri::command]
pub async fn preview_agent_settings(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: AgentSettingsInput,
) -> Result<AgentOutcome, DesktopFailure> {
    main_window(&window)?;
    let broker = window.state::<WebConfirmationState>();
    let active = || window.is_visible().is_ok();
    let visible = || window.is_visible().unwrap_or(false);
    let notify = |request: &WebConfirmationRequest| {
        window
            .emit_to(
                tauri::EventTarget::webview(window.label()),
                "hiroute-web-confirmation",
                request,
            )
            .map_err(|_| "CONFIRMATION_UNAVAILABLE".to_owned())
    };
    let context = hiroute_product_session::commands::ClientContext {
        id: window.label(),
        confirmations: &broker,
        active: &active,
        visible: &visible,
        notify: Some(&notify),
    };
    let product_state = hiroute_product_session::commands::CommandState(
        &state.0,
        &state.1.error,
        &state.2,
        &state.3,
    );
    hiroute_product_session::commands::preview_agent_settings(&context, &product_state, input).await
}
#[tauri::command]
pub async fn check_agent_authentication(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: AgentCheckInput,
) -> Result<bool, DesktopFailure> {
    main_window(&window)?;
    let broker = window.state::<WebConfirmationState>();
    let active = || window.is_visible().is_ok();
    let visible = || window.is_visible().unwrap_or(false);
    let notify = |request: &WebConfirmationRequest| {
        window
            .emit_to(
                tauri::EventTarget::webview(window.label()),
                "hiroute-web-confirmation",
                request,
            )
            .map_err(|_| "CONFIRMATION_UNAVAILABLE".to_owned())
    };
    let context = hiroute_product_session::commands::ClientContext {
        id: window.label(),
        confirmations: &broker,
        active: &active,
        visible: &visible,
        notify: Some(&notify),
    };
    let product_state = hiroute_product_session::commands::CommandState(
        &state.0,
        &state.1.error,
        &state.2,
        &state.3,
    );
    hiroute_product_session::commands::check_agent_authentication(&context, &product_state, input)
        .await
}
#[tauri::command]
pub async fn check_agent_live(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: AgentCheckInput,
) -> Result<AgentCheckCompletion, DesktopFailure> {
    main_window(&window)?;
    let broker = window.state::<WebConfirmationState>();
    let active = || window.is_visible().is_ok();
    let visible = || window.is_visible().unwrap_or(false);
    let notify = |request: &WebConfirmationRequest| {
        window
            .emit_to(
                tauri::EventTarget::webview(window.label()),
                "hiroute-web-confirmation",
                request,
            )
            .map_err(|_| "CONFIRMATION_UNAVAILABLE".to_owned())
    };
    let context = hiroute_product_session::commands::ClientContext {
        id: window.label(),
        confirmations: &broker,
        active: &active,
        visible: &visible,
        notify: Some(&notify),
    };
    let product_state = hiroute_product_session::commands::CommandState(
        &state.0,
        &state.1.error,
        &state.2,
        &state.3,
    );
    hiroute_product_session::commands::check_agent_live(&context, &product_state, input).await
}
#[tauri::command]
pub async fn observe_operation(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<Option<ClientOperationViewV1>, DesktopFailure> {
    main_window(&window)?;
    let broker = window.state::<WebConfirmationState>();
    let active = || window.is_visible().is_ok();
    let visible = || window.is_visible().unwrap_or(false);
    let notify = |request: &WebConfirmationRequest| {
        window
            .emit_to(
                tauri::EventTarget::webview(window.label()),
                "hiroute-web-confirmation",
                request,
            )
            .map_err(|_| "CONFIRMATION_UNAVAILABLE".to_owned())
    };
    let context = hiroute_product_session::commands::ClientContext {
        id: window.label(),
        confirmations: &broker,
        active: &active,
        visible: &visible,
        notify: Some(&notify),
    };
    let product_state = hiroute_product_session::commands::CommandState(
        &state.0,
        &state.1.error,
        &state.2,
        &state.3,
    );
    hiroute_product_session::commands::observe_operation(&context, &product_state).await
}
#[tauri::command]
pub async fn stop_observing(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<(), String> {
    main_window(&window)?;
    let broker = window.state::<WebConfirmationState>();
    let active = || window.is_visible().is_ok();
    let visible = || window.is_visible().unwrap_or(false);
    let notify = |request: &WebConfirmationRequest| {
        window
            .emit_to(
                tauri::EventTarget::webview(window.label()),
                "hiroute-web-confirmation",
                request,
            )
            .map_err(|_| "CONFIRMATION_UNAVAILABLE".to_owned())
    };
    let context = hiroute_product_session::commands::ClientContext {
        id: window.label(),
        confirmations: &broker,
        active: &active,
        visible: &visible,
        notify: Some(&notify),
    };
    let product_state = hiroute_product_session::commands::CommandState(
        &state.0,
        &state.1.error,
        &state.2,
        &state.3,
    );
    hiroute_product_session::commands::stop_observing(&context, &product_state).await
}
pub fn run() {
    // An explicit background start runs the same Resident initialization and recovery chain;
    // only the window presentation differs.
    let background = std::env::args().any(|argument| argument == "--background");
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init());
    #[cfg(all(feature = "desktop-pilot", debug_assertions))]
    let builder = builder.plugin(tauri_plugin_pilot::init());
    let app = builder
        .setup(move |app| {
            let root =
                desktop_data_root(app.handle()).map_err(|_| "PRIVATE_PATH_UNAVAILABLE".to_owned());
            // Managed before the business Session so status and level commands stay responsive
            // while the daemon is still starting, unreachable or crashed. An unusable root
            // degrades this report and never blocks the product.
            let diagnostics = DiagnosticsState::start(root.as_deref().ok());
            // Only this executable entry installs the process-wide hook; library users and
            // test processes never replace the global panic hook.
            hiroute_diagnostics::panic::install_panic_hook(diagnostics.handle());
            let native_diagnostics = diagnostics.startup();
            let diagnostic_handle = diagnostics.handle();
            app.manage(diagnostics);
            app.manage(WebConfirmationState::default());
            app.manage(WorkerDependencyConfirmationState::default());
            #[cfg(target_os = "macos")]
            app.manage(crate::updates::UpdateState::new(
                app.config()
                    .version
                    .clone()
                    .unwrap_or_else(|| env!("CARGO_PKG_VERSION").into()),
                root.clone().ok(),
            ));
            // The single-host desktop.lock is taken before Resident initialization.
            // The configured window is still hidden: a duplicate hands off instead of
            // presenting dead UI. Every
            // other acquisition failure keeps its existing async startup diagnostics.
            #[cfg(unix)]
            let (preacquired, duplicate) = match root.as_deref() {
                Ok(root) => match crate::bootstrap::acquire_host_lock(root) {
                    Ok(lock) => (Some(lock), false),
                    Err(error) => (None, error == "DESKTOP_ALREADY_RUNNING"),
                },
                Err(_) => (None, false),
            };
            #[cfg(not(unix))]
            let (preacquired, duplicate) = (None, false);
            #[cfg(unix)]
            if duplicate {
                crate::duplicate_host::finish(app.handle(), background);
            }
            app.manage(LaunchPresentation::new(app.handle(), background, duplicate));
            app.manage(start_session(
                diagnostic_handle,
                root.clone().ok(),
                move |cancelled, upgrade_progress| {
                    if duplicate {
                        // The duplicate never touches the lock or starts a daemon again; its only
                        // job was the handoff above.
                        return Err("DESKTOP_ALREADY_RUNNING".into());
                    }
                    let root = root?;
                    let native_diagnostics =
                        native_diagnostics.with_upgrade_progress(upgrade_progress);
                    let diagnostics = &native_diagnostics;
                    let binary = std::env::current_exe()
                        .map_err(|_| "DAEMON_BINARY_UNAVAILABLE")?
                        .with_file_name("hirouted");
                    #[cfg(unix)]
                    {
                        Ok(crate::session_from_resident(
                            crate::bootstrap::Resident::open_resident(
                                &root,
                                &binary,
                                cancelled,
                                diagnostics,
                                preacquired,
                            )?,
                        ))
                    }
                    #[cfg(not(unix))]
                    {
                        let _ = (root, binary, cancelled, diagnostics, preacquired);
                        Err("UNSUPPORTED_PLATFORM".into())
                    }
                },
            ));
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                expire_window_confirmations(window.app_handle(), window.label());
                api.prevent_close();
                let _ = window.hide();
                let state = window.state::<DesktopState>().0.clone();
                let epoch = window.state::<DesktopState>().2.load(Ordering::SeqCst);
                let handle = window.app_handle().clone();
                tauri::async_runtime::spawn(async move {
                    let mut guard = state.lock().await;
                    if handle.state::<DesktopState>().2.load(Ordering::SeqCst) != epoch {
                        return;
                    }
                    if let Some(session) = guard.as_mut() {
                        session.invalidate_confirmation();
                        session.stop_observing();
                        let _ = session.close_subscription_check().await;
                    }
                });
            }
        })
        .invoke_handler(tauri::generate_handler![
            observation_read,
            observation_delete,
            observation_capacity_get,
            observation_capacity_set,
            test_classifier_decision,
            decision_services,
            save_decision_service,
            decision_connection_sources,
            decision_services_v2,
            save_decision_service_v2,
            test_decision_service_v2,
            test_decision_service_v3,
            save_classifier_header_secret,
            save_classifier_openapi,
            desktop_snapshot,
            preview_rename,
            preview_plan_editor,
            refresh_route_references,
            plan_editor_options,
            preview_restore_name,
            preview_price_change,
            model_reference_query,
            model_feed_status,
            model_feed_refresh,
            model_feed_settings_apply,
            model_feed_remove_preview,
            model_feed_remove_apply,
            model_availability,
            list_model_metadata,
            get_model_metadata,
            compute_defaults,
            preview_compute_save_v3,
            native_source_network,
            preview_native_network_route,
            preview_native_source_network,
            apply_native_source_network,
            preview_compute_save_v4,
            check_model_connection_v2,
            check_registered_model_connection_v2,
            check_saved_model_connection_v2,
            effective_price_query,
            compute_management_snapshot,
            compute_key_masks,
            compute_scan,
            prepare_discovered_model_connection,
            compute_connection_options,
            register_protected_model_input,
            release_protected_model_input,
            check_model_connection,
            check_registered_model_connection,
            check_saved_model_connection,
            cancel_model_connection_check,
            preview_compute_save,
            apply_compute_save,
            get_compute_save_result,
            compute_subscriptions,
            manage_subscription_login,
            submit_subscription_login_callback,
            check_subscription,
            get_subscription_check_result,
            recover_subscription_check,
            close_subscription_check,
            release_subscription_check,
            web_confirmation_snapshot,
            resolve_web_confirmation,
            agent_snapshot,
            trial_setup_snapshot,
            agent_home_summary,
            agent_model_associations,
            trial_agent_choices,
            trial_agent_model,
            preview_agent_previous_routing,
            preview_agent_settings,
            retry_agent_settings,
            check_agent_authentication,
            check_agent_live,
            worker_settings_get,
            worker_settings_set,
            worker_task_plans,
            worker_executor_availability,
            worker_dependencies_discover,
            worker_dependencies_select_prepare,
            worker_dependencies_select_confirm,
            worker_dependencies_select_cancel,
            worker_task_list,
            worker_task_status,
            worker_task_result,
            worker_task_read,
            worker_task_wait,
            worker_task_cancel,
            worker_task_confirm_residual,
            worker_task_continue,
            cli_entry_status,
            cli_entry_install,
            cli_entry_remove,
            gateway_listener_status,
            subscription_proxy_status,
            subscription_proxy_apply,
            gateway_listener_apply,
            gateway_listener_recover,
            observe_operation,
            stop_observing,
            quit_desktop,
            diagnostic_status,
            startup_status,
            recover_native_connection,
            open_startup_recovery_directory,
            update_status,
            update_check,
            update_download,
            update_install,
            update_cancel,
            set_diagnostic_level,
            open_diagnostic_directory,
            open_external_url,
            perform_titlebar_double_click,
        ])
        .build(tauri::generate_context!())
        .expect("build HiRoute Desktop");
    launch_presentation::observe_before_run(&app, background);
    app.run(|handle, event| {
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Reopen { .. } = event
            && let Some(window) = handle.get_webview_window("main")
        {
            let _ = window.show();
            let _ = window.set_focus();
        }
        if let tauri::RunEvent::Ready = event {
            handle.state::<LaunchPresentation>().on_ready(handle);
        }
        shutdown::on_run_event(handle, &event);
    });
}

/// The explicit Quit path. The WebView has already shown the "configured Agents are
/// temporarily unavailable" prompt; exiting stops the owned daemon and this host through
/// the same shutdown path as any other termination, leaving user configuration and the
/// owned login item untouched. A later manual start or login restores the service.
#[tauri::command]
pub async fn quit_desktop(window: WebviewWindow) -> Result<(), DesktopFailure> {
    main_window(&window)?;
    window.app_handle().exit(0);
    Ok(())
}

#[tauri::command]
pub async fn observation_read(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: hiroute_application_api::ObservationReadRequestV2,
) -> Result<serde_json::Value, DesktopFailure> {
    main_window(&window)?;
    let broker = window.state::<WebConfirmationState>();
    let active = || window.is_visible().is_ok();
    let visible = || window.is_visible().unwrap_or(false);
    let notify = |request: &WebConfirmationRequest| {
        window
            .emit_to(
                tauri::EventTarget::webview(window.label()),
                "hiroute-web-confirmation",
                request,
            )
            .map_err(|_| "CONFIRMATION_UNAVAILABLE".to_owned())
    };
    let context = hiroute_product_session::commands::ClientContext {
        id: window.label(),
        confirmations: &broker,
        active: &active,
        visible: &visible,
        notify: Some(&notify),
    };
    let product_state = hiroute_product_session::commands::CommandState(
        &state.0,
        &state.1.error,
        &state.2,
        &state.3,
    );
    hiroute_product_session::commands::observation_read(&context, &product_state, request).await
}

#[cfg(test)]
#[path = "bridge_command_acl_tests.rs"]
mod command_acl_tests;
