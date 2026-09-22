//! WebView supplies explicit intent and renders prompts; native preview and typed policy own submission.
use crate::failure::DesktopFailure;
use crate::session::{
    AgentCheckCompletion, AgentCheckConfirmation, AgentCheckDispatch, AgentCheckInput,
    AgentConfirmation, AgentOutcome, AgentPreparation, AgentSettingsInput, AgentSnapshot,
    Confirmation, DesktopSnapshot, EditorInput, ModelSaveAccepted, ModelSaveConfirmation,
    MutationOutcome, PriceConfirmation, PriceDisplayResult, PriceEditInput, RenameInput,
    RestoreInput, Session, SubscriptionCheckConfirmation,
};
use hiroute_application_api::{
    ClientOperationViewV1, GetEffectivePricesV2, ModelCatalogQueryV1, ModelCatalogResultV1,
};
use hiroute_diagnostics::context::{DiagnosticContext, DiagnosticHandle};
use hiroute_diagnostics::correlation::CorrelationDomain;
use hiroute_diagnostics::error::EventErrorCode;
use hiroute_diagnostics::event::{
    ActionEnd, ActionOperation, ActionResult, ApplyEvent, ApplyPhase, ConfirmationEvent,
    ConfirmationPhase, DiagnosticEvent, PreviewEvent, PreviewPhase,
};
use hiroute_diagnostics::identity::CorrelationToken;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use tauri::{Manager, State, WebviewWindow};
use tokio::sync::Mutex;
mod model_connection_web;
mod model_connections;
use model_connections::*;
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
use host_settings::*;
mod startup;
pub use diagnostics::DiagnosticsState;
pub use startup::StartupState;
use startup::*;
mod web_confirmation;
use web_confirmation::*;
mod window_behavior;
use window_behavior::*;
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

/// Diagnostics for one protected native action. Carries the operation family, the real
/// revision and stable results; never confirmation text, payloads, digests or grants.
struct ActionDiagnostics {
    context: DiagnosticContext,
    operation: ActionOperation,
    started: std::time::Instant,
}

impl ActionDiagnostics {
    fn begin(handle: &DiagnosticHandle, operation: ActionOperation) -> Self {
        Self {
            context: handle.context(),
            operation,
            started: std::time::Instant::now(),
        }
    }

    fn preview(&self, phase: PreviewPhase, revision: Option<u64>) {
        self.context.emit(DiagnosticEvent::Preview(PreviewEvent {
            operation: self.operation,
            phase,
            revision,
        }));
    }

    fn confirmation(&self, phase: ConfirmationPhase, accepted: Option<bool>) {
        self.context
            .emit(DiagnosticEvent::Confirmation(ConfirmationEvent {
                operation: self.operation,
                phase,
                accepted,
            }));
    }

    fn apply(&self, phase: ApplyPhase, revision: Option<u64>, result: Option<ActionResult>) {
        self.context.emit(DiagnosticEvent::Apply(ApplyEvent {
            operation: self.operation,
            phase,
            revision,
            result,
        }));
    }

    /// The exact operation this action created, when the product reported one.
    fn token(&self, operation_id: Option<&str>) -> Option<CorrelationToken> {
        operation_id.and_then(|id| self.context.token(CorrelationDomain::Operation, id))
    }

    fn end(&self, result: ActionResult, operation_token: Option<CorrelationToken>) {
        self.context.emit(DiagnosticEvent::ActionEnd(ActionEnd {
            operation: self.operation,
            result,
            operation_token,
            elapsed_ms: self.started.elapsed().as_millis() as u64,
        }));
    }
}

impl NativeOutcome {
    /// The stable result of this action; a denial is not a failure and vice versa. A
    /// non-terminal product state is reported as a failure instead of "applied".
    fn action_result(&self, accepted: bool) -> ActionResult {
        if !accepted {
            return ActionResult::Denied;
        }
        match self {
            Self::Rename(mutation) => mutation_outcome_result(mutation),
            Self::Agent(agent) => match &agent.mutation {
                Some(mutation) => mutation_outcome_result(mutation),
                None => ActionResult::Failed {
                    code: EventErrorCode::ExternalError,
                },
            },
            Self::Check(completion) => {
                if completion.passed() {
                    ActionResult::Applied
                } else {
                    ActionResult::Failed {
                        code: EventErrorCode::ExternalError,
                    }
                }
            }
            Self::ModelSave(accepted) => mutation_result(&accepted.result.state),
            Self::SubscriptionCheck(result) => match result.status {
                hiroute_application_api::ComputeSubscriptionCheckStatusV2::Verified => {
                    ActionResult::Applied
                }
                hiroute_application_api::ComputeSubscriptionCheckStatusV2::SourceChanged => {
                    ActionResult::Failed {
                        code: EventErrorCode::Conflict,
                    }
                }
                hiroute_application_api::ComputeSubscriptionCheckStatusV2::NeedsAuth => {
                    ActionResult::Failed {
                        code: EventErrorCode::Authorization,
                    }
                }
                hiroute_application_api::ComputeSubscriptionCheckStatusV2::Unavailable => {
                    ActionResult::Failed {
                        code: EventErrorCode::Unavailable,
                    }
                }
                _ => ActionResult::Failed {
                    code: EventErrorCode::ExternalError,
                },
            },
        }
    }

    /// The exact product operation this action produced, when it reported one.
    fn operation_id(&self) -> Option<&str> {
        match self {
            Self::Rename(mutation) => mutation
                .operation
                .as_ref()
                .map(|operation| operation.operation_id.as_str()),
            Self::Agent(agent) => agent
                .mutation
                .as_ref()
                .and_then(|mutation| mutation.operation.as_ref())
                .map(|operation| operation.operation_id.as_str()),
            Self::ModelSave(accepted) => Some(accepted.operation.operation_id.as_str()),
            Self::SubscriptionCheck(result) => {
                Some(result.approval_operation.operation_id.as_str())
            }
            Self::Check(_) => None,
        }
    }
}

fn mutation_outcome_result(mutation: &MutationOutcome) -> ActionResult {
    mutation.operation.as_ref().map_or_else(
        || mutation_result(&mutation.state),
        |operation| mutation_result(&operation.state),
    )
}

fn mutation_result(state: &str) -> ActionResult {
    match state {
        "succeeded" => ActionResult::Applied,
        "needs_attention" => ActionResult::Failed {
            code: EventErrorCode::Recovery,
        },
        _ => ActionResult::Failed {
            code: EventErrorCode::ExternalError,
        },
    }
}
#[tauri::command]
pub async fn desktop_snapshot(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<DesktopSnapshot, DesktopFailure> {
    main_window(&window)?;
    let mut guard = state.0.lock().await;
    let session = guard.as_mut().ok_or_else(|| {
        state
            .1
            .error
            .get()
            .cloned()
            .unwrap_or("RESIDENT_UNAVAILABLE".into())
    })?;
    session.resume_observing();
    session.snapshot().await
}
#[tauri::command]
pub async fn preview_plan_editor(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: EditorInput,
) -> Result<MutationOutcome, DesktopFailure> {
    main_window(&window)?;
    let epoch = state.2.load(Ordering::SeqCst);
    let action = ActionDiagnostics::begin(&state.3, ActionOperation::PlanEditor);
    action.preview(PreviewPhase::Begin, None);
    let context = state
        .0
        .lock()
        .await
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .preview_editor(input)
        .await?;
    action.preview(PreviewPhase::End, Some(context.revision()));
    confirm(
        &window,
        &state,
        ActionOperation::PlanEditor,
        NativeConfirmation::Rename(Box::new(context)),
        epoch,
    )
    .await?
    .mutation()
}
#[tauri::command]
pub async fn plan_editor_options(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: hiroute_application_api::PlanEditorOptionsRequestV1,
) -> Result<hiroute_application_api::PlanEditorOptionsV1, DesktopFailure> {
    main_window(&window)?;
    let guard = state.0.lock().await;
    let session = guard.as_ref().ok_or("RESIDENT_UNAVAILABLE")?;
    crate::session::query(&session.client, "GetPlanEditorOptions", &input).await
}
#[tauri::command]
pub async fn preview_rename(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: RenameInput,
) -> Result<MutationOutcome, DesktopFailure> {
    main_window(&window)?;
    let epoch = state.2.load(Ordering::SeqCst);
    let action = ActionDiagnostics::begin(&state.3, ActionOperation::Rename);
    action.preview(PreviewPhase::Begin, None);
    let context = {
        let mut guard = state.0.lock().await;
        guard
            .as_mut()
            .ok_or("RESIDENT_UNAVAILABLE")?
            .preview(input)
            .await?
    };
    action.preview(PreviewPhase::End, Some(context.revision()));
    confirm(
        &window,
        &state,
        ActionOperation::Rename,
        NativeConfirmation::Rename(Box::new(context)),
        epoch,
    )
    .await?
    .mutation()
}
#[tauri::command]
pub async fn preview_restore_name(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: RestoreInput,
) -> Result<MutationOutcome, DesktopFailure> {
    main_window(&window)?;
    let epoch = state.2.load(Ordering::SeqCst);
    let action = ActionDiagnostics::begin(&state.3, ActionOperation::RestoreName);
    action.preview(PreviewPhase::Begin, None);
    let context = state
        .0
        .lock()
        .await
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .preview_restore(input)
        .await?;
    action.preview(PreviewPhase::End, Some(context.revision()));
    confirm(
        &window,
        &state,
        ActionOperation::RestoreName,
        NativeConfirmation::Rename(Box::new(context)),
        epoch,
    )
    .await?
    .mutation()
}
enum NativeConfirmation {
    Rename(Box<Confirmation>),
    Price(Box<PriceConfirmation>),
    Agent(Box<AgentConfirmation>),
    Check(Box<AgentCheckConfirmation>),
    ModelSave(Box<ModelSaveConfirmation>),
    SubscriptionCheck(Box<SubscriptionCheckConfirmation>),
}
enum NativeOutcome {
    Rename(MutationOutcome),
    Agent(Box<AgentOutcome>),
    Check(AgentCheckCompletion),
    ModelSave(ModelSaveAccepted),
    SubscriptionCheck(hiroute_application_api::ComputeSubscriptionCheckResultV2),
}
impl NativeOutcome {
    fn mutation(self) -> Result<MutationOutcome, DesktopFailure> {
        match self {
            Self::Rename(value) => Ok(value),
            _ => Err("CONFIRMATION_KIND_MISMATCH".into()),
        }
    }
}
impl NativeConfirmation {
    /// The exact revision this action was previewed against, from the real confirmation
    /// context rather than from any WebView-provided value.
    fn revision(&self) -> u64 {
        match self {
            Self::Rename(context) => context.revision(),
            Self::Price(context) => context.revision(),
            Self::Agent(context) => context.revision(),
            Self::Check(context) => context.revision(),
            Self::ModelSave(context) => context.revision(),
            Self::SubscriptionCheck(context) => context.revision(),
        }
    }

    fn requires_confirmation(&self) -> bool {
        match self {
            Self::Rename(c) => c.requires_confirmation(),
            Self::Price(_) => false,
            Self::Check(c) => c.requires_confirmation(),
            Self::Agent(c) => c.requires_confirmation(),
            Self::ModelSave(c) => c.requires_confirmation(),
            Self::SubscriptionCheck(c) => c.requires_confirmation(),
        }
    }
    fn english(&self) -> bool {
        match self {
            Self::Rename(c) => c.english(),
            Self::Price(c) => c.english(),
            Self::Agent(c) => c.english(),
            Self::Check(c) => c.english(),
            Self::ModelSave(c) => c.english(),
            Self::SubscriptionCheck(c) => c.english(),
        }
    }
    fn message(&self) -> String {
        match self {
            Self::Rename(c) => c.message(),
            Self::Price(c) => c.message(),
            Self::Agent(c) => c.message(),
            Self::Check(c) => c.message(),
            Self::ModelSave(c) => c.message(),
            Self::SubscriptionCheck(c) => c.message(),
        }
    }
}
#[tauri::command]
pub async fn preview_price_change(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: PriceEditInput,
) -> Result<MutationOutcome, DesktopFailure> {
    main_window(&window)?;
    let epoch = state.2.load(Ordering::SeqCst);
    let action = ActionDiagnostics::begin(&state.3, ActionOperation::PriceChange);
    action.preview(PreviewPhase::Begin, None);
    let context = state
        .0
        .lock()
        .await
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .preview_price(input)
        .await?;
    action.preview(PreviewPhase::End, Some(context.revision()));
    confirm(
        &window,
        &state,
        ActionOperation::PriceChange,
        NativeConfirmation::Price(Box::new(context)),
        epoch,
    )
    .await?
    .mutation()
}
#[tauri::command]
pub async fn model_reference_query(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: ModelCatalogQueryV1,
) -> Result<ModelCatalogResultV1, DesktopFailure> {
    main_window(&window)?;
    let guard = state.0.lock().await;
    let session = guard.as_ref().ok_or("RESIDENT_UNAVAILABLE")?;
    crate::session::query(&session.client, "ShowModel", &input).await
}
#[tauri::command]
pub async fn effective_price_query(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: GetEffectivePricesV2,
) -> Result<PriceDisplayResult, DesktopFailure> {
    main_window(&window)?;
    let guard = state.0.lock().await;
    let session = guard.as_ref().ok_or("RESIDENT_UNAVAILABLE")?;
    Ok(PriceDisplayResult::new(
        crate::session::query(&session.client, "GetEffectivePrices", &input).await?,
    ))
}

#[tauri::command]
pub async fn agent_snapshot(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<AgentSnapshot, DesktopFailure> {
    main_window(&window)?;
    state
        .0
        .lock()
        .await
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .agent_snapshot()
        .await
}
#[tauri::command]
pub async fn preview_agent_settings(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: AgentSettingsInput,
) -> Result<AgentOutcome, DesktopFailure> {
    main_window(&window)?;
    let epoch = state.2.load(Ordering::SeqCst);
    let action = ActionDiagnostics::begin(&state.3, ActionOperation::AgentSettings);
    action.preview(PreviewPhase::Begin, None);
    let prepared = state
        .0
        .lock()
        .await
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .preview_agent_settings(input)
        .await?;
    match prepared {
        AgentPreparation::Blocked(preview) => {
            action.preview(PreviewPhase::End, None);
            action.end(
                ActionResult::Failed {
                    code: EventErrorCode::ActionRequired,
                },
                None,
            );
            Ok(AgentOutcome {
                preview: *preview,
                mutation: None,
            })
        }
        AgentPreparation::Ready(context) => {
            action.preview(PreviewPhase::End, Some(context.revision()));
            match confirm(
                &window,
                &state,
                ActionOperation::AgentSettings,
                NativeConfirmation::Agent(context),
                epoch,
            )
            .await?
            {
                NativeOutcome::Agent(value) => Ok(*value),
                _ => unreachable!(),
            }
        }
    }
}
#[tauri::command]
pub async fn check_agent_authentication(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: AgentCheckInput,
) -> Result<bool, DesktopFailure> {
    main_window(&window)?;
    let epoch = state.2.load(Ordering::SeqCst);
    let action = ActionDiagnostics::begin(&state.3, ActionOperation::AgentCheck);
    action.preview(PreviewPhase::Begin, None);
    let context = state
        .0
        .lock()
        .await
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .prepare_agent_check(input)
        .await?;
    action.preview(PreviewPhase::End, Some(context.revision()));
    match confirm(
        &window,
        &state,
        ActionOperation::AgentCheck,
        NativeConfirmation::Check(Box::new(context)),
        epoch,
    )
    .await?
    {
        NativeOutcome::Check(result) => Ok(result.passed()),
        _ => unreachable!(),
    }
}

#[tauri::command]
pub async fn check_agent_live(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: AgentCheckInput,
) -> Result<AgentCheckCompletion, DesktopFailure> {
    main_window(&window)?;
    if input.scope.as_deref() != Some("live") {
        return Err("AGENT_INPUT_INVALID".into());
    }
    let epoch = state.2.load(Ordering::SeqCst);
    let action = ActionDiagnostics::begin(&state.3, ActionOperation::AgentCheck);
    action.preview(PreviewPhase::Begin, None);
    let context = state
        .0
        .lock()
        .await
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .prepare_agent_check(input)
        .await?;
    action.preview(PreviewPhase::End, Some(context.revision()));
    match confirm(
        &window,
        &state,
        ActionOperation::AgentCheck,
        NativeConfirmation::Check(Box::new(context)),
        epoch,
    )
    .await?
    {
        NativeOutcome::Check(result) => Ok(result),
        _ => unreachable!(),
    }
}

async fn confirm(
    window: &WebviewWindow,
    state: &DesktopState,
    operation: ActionOperation,
    context: NativeConfirmation,
    epoch: u64,
) -> Result<NativeOutcome, DesktopFailure> {
    let action = ActionDiagnostics::begin(&state.3, operation);
    if state.2.load(Ordering::SeqCst) != epoch || !window.is_visible().unwrap_or(false) {
        state
            .0
            .lock()
            .await
            .as_mut()
            .ok_or("RESIDENT_UNAVAILABLE")?
            .invalidate_confirmation();
        action.confirmation(ConfirmationPhase::Expired, None);
        action.end(ActionResult::Stale, None);
        return Err("CONFIRMATION_STALE".into());
    }
    let revision = context.revision();
    let accepted = if context.requires_confirmation() {
        action.confirmation(ConfirmationPhase::Shown, None);
        let english = context.english();
        let checking = matches!(&context, NativeConfirmation::Check(_));
        let live_checking = matches!(&context, NativeConfirmation::Check(c) if c.is_live());
        let title = if live_checking {
            if english {
                "HiRoute · Verify model access"
            } else {
                "HiRoute · 验证模型接入"
            }
        } else if checking {
            if english {
                "HiRoute · Check compatibility"
            } else {
                "HiRoute · 检查兼容性"
            }
        } else if english {
            "HiRoute · Confirm change"
        } else {
            "HiRoute · 确认更改"
        };
        let confirm_label = if live_checking {
            if english {
                "Run live check"
            } else {
                "开始真实验证"
            }
        } else if checking {
            if english {
                "Start check"
            } else {
                "开始检查"
            }
        } else if let NativeConfirmation::Rename(plan) = &context {
            plan.action_label()
        } else if english {
            "Apply change"
        } else {
            "应用更改"
        };
        let accepted = request_web_confirmation(
            window,
            WebConfirmationPrompt {
                title: title.into(),
                message: context.message(),
                confirm_label: confirm_label.into(),
                cancel_label: if english { "Cancel" } else { "取消" }.into(),
            },
        )
        .await;
        action.confirmation(ConfirmationPhase::Result, Some(accepted));
        accepted
    } else {
        // This context was freshly validated for an explicit action that needs no extra prompt.
        true
    };
    if state.2.load(Ordering::SeqCst) != epoch || !window.is_visible().unwrap_or(false) {
        state
            .0
            .lock()
            .await
            .as_mut()
            .ok_or("RESIDENT_UNAVAILABLE")?
            .invalidate_confirmation();
        // The approval was invalidated; the caller's existing semantics still decide what
        // happens next, and diagnostics only record that the confirmation expired.
        action.confirmation(ConfirmationPhase::Expired, None);
    }
    action.apply(ApplyPhase::Begin, Some(revision), None);
    let outcome = if let NativeConfirmation::SubscriptionCheck(context) = context {
        let pending = {
            let mut guard = state.0.lock().await;
            guard
                .as_mut()
                .ok_or("RESIDENT_UNAVAILABLE")?
                .begin_subscription_check_confirmation(*context, accepted)
                .await?
        };
        // Operation A performs connector I/O. Keep native close/recovery commands responsive while
        // it runs so close can submit cancellation rather than waiting behind this command.
        let completion = pending.execute().await;
        state
            .0
            .lock()
            .await
            .as_mut()
            .ok_or("RESIDENT_UNAVAILABLE")?
            .finish_subscription_check_confirmation(completion)
            .await
            .map(NativeOutcome::SubscriptionCheck)
    } else if let NativeConfirmation::Check(context) = context {
        let dispatch = {
            let mut guard = state.0.lock().await;
            guard
                .as_mut()
                .ok_or("RESIDENT_UNAVAILABLE")?
                .begin_agent_check(*context, accepted)?
        };
        match dispatch {
            AgentCheckDispatch::Cancelled(result) => Ok(NativeOutcome::Check(result)),
            AgentCheckDispatch::Pending(pending) => {
                pending.execute().await.map(NativeOutcome::Check)
            }
        }
    } else {
        let mut guard = state.0.lock().await;
        let session = guard.as_mut().ok_or("RESIDENT_UNAVAILABLE")?;
        match context {
            NativeConfirmation::Agent(context) => session
                .finish_agent_confirmation(*context, accepted)
                .await
                .map(|value| NativeOutcome::Agent(Box::new(value))),
            NativeConfirmation::Check(_) => unreachable!(),
            NativeConfirmation::Rename(context) => session
                .finish_native_confirmation(*context, accepted)
                .await
                .map(NativeOutcome::Rename),
            NativeConfirmation::Price(context) => session
                .finish_price_confirmation(*context, accepted)
                .await
                .map(NativeOutcome::Rename),
            NativeConfirmation::ModelSave(context) => session
                .finish_model_save_confirmation(*context, accepted)
                .await
                .map(NativeOutcome::ModelSave),
            NativeConfirmation::SubscriptionCheck(_) => unreachable!(),
        }
    };
    match &outcome {
        Ok(value) => {
            let result = value.action_result(accepted);
            let token = action.token(value.operation_id());
            action.apply(ApplyPhase::End, Some(revision), Some(result));
            action.end(result, token);
        }
        Err(failure) => {
            let result = ActionResult::Failed {
                code: failure.event_code(),
            };
            action.apply(ApplyPhase::End, Some(revision), Some(result));
            action.end(result, None);
        }
    }
    outcome
}
#[tauri::command]
pub async fn observe_operation(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<Option<ClientOperationViewV1>, DesktopFailure> {
    main_window(&window)?;
    state
        .0
        .lock()
        .await
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .observe()
        .await
}
#[tauri::command]
pub async fn stop_observing(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<(), String> {
    main_window(&window)?;
    state
        .0
        .lock()
        .await
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .stop_observing();
    Ok(())
}
/// How the main window may be presented at launch. The window is created hidden; only a
/// normal foreground launch shows it, and only after the native launch reason is known.
pub struct LaunchPresentation {
    background: bool,
    duplicate: bool,
    /// No native launch-event observer exists; a foreground launch shows at Ready.
    fallback_at_ready: bool,
}

impl LaunchPresentation {
    fn headless(&self) -> bool {
        self.background || self.duplicate
    }

    fn present_main_window(&self, handle: &tauri::AppHandle) {
        if self.headless() {
            return;
        }
        if let Some(window) = handle.get_webview_window("main")
            && !window.is_visible().unwrap_or(false)
        {
            let _ = window.show();
        }
    }
}

pub fn run() {
    // An explicit background start runs the same Resident initialization and recovery chain;
    // only the window presentation differs.
    let background = std::env::args().any(|argument| argument == "--background");
    let builder = tauri::Builder::default().plugin(tauri_plugin_dialog::init());
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
            // The single-host desktop.lock is taken synchronously, before any window exists:
            // a duplicate host hands off and exits here instead of presenting dead UI. Every
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
            // The native launch-event observer is installed before any window can be shown,
            // so a login-item launch initializes the same Resident without a flash of UI.
            #[cfg(target_os = "macos")]
            let native_observer = if background || duplicate {
                false
            } else {
                let handle = app.handle().clone();
                hiroute_desktop_host_effects::observe_launch_reason(move |login_item| {
                    if !login_item {
                        handle
                            .state::<LaunchPresentation>()
                            .present_main_window(&handle);
                    }
                })
            };
            app.manage(LaunchPresentation {
                background,
                duplicate,
                fallback_at_ready: !background && !duplicate && {
                    #[cfg(target_os = "macos")]
                    {
                        // Pilot launches a bare Debug executable, which does not receive the
                        // normal open-application Apple event. Present its isolated test window
                        // at Ready so protected confirmations can use a visible main window.
                        !native_observer || cfg!(all(feature = "desktop-pilot", debug_assertions))
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        true
                    }
                },
            });
            app.manage(start_session(
                diagnostic_handle,
                root.clone().ok(),
                move |cancelled| {
                    if duplicate {
                        // The duplicate never touches the lock or starts a daemon again; its only
                        // job was the handoff above.
                        return Err("DESKTOP_ALREADY_RUNNING".into());
                    }
                    let root = root?;
                    let diagnostics = &native_diagnostics;
                    let binary = std::env::current_exe()
                        .map_err(|_| "DAEMON_BINARY_UNAVAILABLE")?
                        .with_file_name("hirouted");
                    #[cfg(unix)]
                    {
                        Ok(Session::new(crate::bootstrap::Resident::open_resident(
                            &root,
                            &binary,
                            cancelled,
                            diagnostics,
                            preacquired,
                        )?))
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
                window
                    .state::<DesktopState>()
                    .2
                    .fetch_add(1, Ordering::SeqCst);
                window
                    .state::<WebConfirmationState>()
                    .cancel_window(window.label());
                window
                    .state::<WorkerDependencyConfirmationState>()
                    .cancel_window(window.label());
                api.prevent_close();
                let _ = window.hide();
                let state = window.state::<DesktopState>().0.clone();
                tauri::async_runtime::spawn(async move {
                    if let Some(session) = state.lock().await.as_mut() {
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
            test_classifier_decision,
            save_classifier_header_secret,
            save_classifier_openapi,
            desktop_snapshot,
            preview_rename,
            preview_plan_editor,
            plan_editor_options,
            preview_restore_name,
            preview_price_change,
            model_reference_query,
            effective_price_query,
            compute_management_snapshot,
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
            check_subscription,
            get_subscription_check_result,
            recover_subscription_check,
            close_subscription_check,
            release_subscription_check,
            web_confirmation_snapshot,
            resolve_web_confirmation,
            agent_snapshot,
            preview_agent_settings,
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
            worker_task_continue,
            cli_entry_status,
            cli_entry_install,
            cli_entry_remove,
            gateway_listener_status,
            gateway_listener_apply,
            gateway_listener_recover,
            observe_operation,
            stop_observing,
            quit_desktop,
            diagnostic_status,
            startup_status,
            open_startup_recovery_directory,
            set_diagnostic_level,
            open_diagnostic_directory,
            open_external_url,
            perform_titlebar_double_click,
        ])
        .build(tauri::generate_context!())
        .expect("build HiRoute Desktop");
    app.run(|handle, event| {
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Reopen { .. } = event
            && let Some(window) = handle.get_webview_window("main")
        {
            let _ = window.show();
            let _ = window.set_focus();
        }
        if let tauri::RunEvent::Ready = event {
            let presentation = handle.state::<LaunchPresentation>();
            if presentation.headless() {
                // A background or duplicate start never presents the window at launch.
            } else if presentation.fallback_at_ready {
                presentation.present_main_window(handle);
            } else {
                // On macOS the launch reason decides: if the native launch event has already
                // been dispatched, its observer recorded the reason before this point;
                // otherwise the observer decides at dispatch. Reinstall it in case something
                // replaced it after setup — the window must never appear before the native
                // launch reason is known.
                #[cfg(target_os = "macos")]
                match hiroute_desktop_host_effects::launch_reason_recorded() {
                    Some(login_item) => {
                        if !login_item {
                            presentation.present_main_window(handle);
                        }
                    }
                    None => hiroute_desktop_host_effects::reinstate_launch_reason_observer(),
                }
            }
        }
        if let tauri::RunEvent::Exit = event {
            let state = handle.state::<DesktopState>();
            state.1.cancelled.store(true, Ordering::SeqCst);
            tauri::async_runtime::block_on(async {
                drop(state.0.lock().await.take());
            });
            // The resident child is gone by now; flush this process's own bounded report.
            handle.state::<DiagnosticsState>().shutdown();
        }
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
    state
        .0
        .lock()
        .await
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .observation_read(request)
        .await
}

#[cfg(test)]
#[path = "bridge_command_acl_tests.rs"]
mod command_acl_tests;

#[cfg(test)]
#[path = "bridge_launch_presentation_tests.rs"]
mod launch_presentation_tests;
