//! WebView supplies explicit intent and renders prompts; native preview and typed policy own submission.
use crate::failure::DesktopFailure;
pub use crate::session::{
    AgentCheckCompletion, AgentCheckConfirmation, AgentCheckDispatch, AgentCheckInput,
    AgentConfirmation, AgentOutcome, AgentPreparation, AgentSettingsInput, AgentSnapshot,
    Confirmation, DesktopSnapshot, EditorInput, ModelSaveAccepted, ModelSaveConfirmation,
    MutationOutcome, PriceConfirmation, PriceDisplayResult, PriceEditInput, RenameInput,
    RestoreInput, Session, SubscriptionCheckConfirmation, TrialAgentChoices, TrialAgentModel,
};
pub use hiroute_application_api::*;
use hiroute_diagnostics::context::{DiagnosticContext, DiagnosticHandle};
use hiroute_diagnostics::correlation::CorrelationDomain;
use hiroute_diagnostics::error::EventErrorCode;
use hiroute_diagnostics::event::{
    ActionEnd, ActionOperation, ActionResult, ApplyEvent, ApplyPhase, ConfirmationEvent,
    ConfirmationPhase, DiagnosticEvent, PreviewEvent, PreviewPhase,
};
use hiroute_diagnostics::identity::CorrelationToken;
pub use hiroute_domain::ComplexityClassifierModeV1;
use serde_json::Value;
use std::sync::atomic::Ordering;
mod model_connection_web;
pub use model_connection_web::*;
mod model_connections;
pub use model_connections::*;
mod model_feed;
pub use model_feed::*;
mod subscription_login;
pub use subscription_login::*;
mod observation;
pub use observation::*;
mod classifier;
pub use classifier::*;
mod worker_tasks;
pub use worker_tasks::*;
mod web_confirmation;
use web_confirmation::*;
mod read_context;
mod trial_agents;
pub use read_context::{desktop_snapshot, observation_read};
pub use trial_agents::{trial_agent_choices, trial_agent_model};
mod context;
pub use context::*;
async fn confirmation_session<'a>(
    state: &'a CommandState<'_>,
    epoch: u64,
) -> Result<tokio::sync::MutexGuard<'a, Option<Session>>, DesktopFailure> {
    crate::confirmation::lock_current_session(state.0, state.2, epoch)
        .await
        .map_err(Into::into)
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
pub async fn refresh_route_references(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    language: String,
) -> Result<MutationOutcome, DesktopFailure> {
    main_window(window)?;
    let epoch = state.2.load(Ordering::SeqCst);
    let action = ActionDiagnostics::begin(state.3, ActionOperation::PlanEditor);
    action.preview(PreviewPhase::Begin, None);
    let context = confirmation_session(state, epoch)
        .await?
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .preview_reference_refresh(language)
        .await?;
    action.preview(PreviewPhase::End, Some(context.revision()));
    confirm(
        window,
        state,
        ActionOperation::PlanEditor,
        NativeConfirmation::Rename(Box::new(context)),
        epoch,
    )
    .await?
    .mutation()
}
pub async fn preview_plan_editor(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    input: EditorInput,
) -> Result<MutationOutcome, DesktopFailure> {
    main_window(window)?;
    let epoch = state.2.load(Ordering::SeqCst);
    let action = ActionDiagnostics::begin(state.3, ActionOperation::PlanEditor);
    action.preview(PreviewPhase::Begin, None);
    let context = confirmation_session(state, epoch)
        .await?
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .preview_editor(input)
        .await?;
    action.preview(PreviewPhase::End, Some(context.revision()));
    confirm(
        window,
        state,
        ActionOperation::PlanEditor,
        NativeConfirmation::Rename(Box::new(context)),
        epoch,
    )
    .await?
    .mutation()
}
pub async fn plan_editor_options(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    input: hiroute_application_api::PlanEditorOptionsRequestV1,
) -> Result<hiroute_application_api::PlanEditorOptionsV1, DesktopFailure> {
    main_window(window)?;
    let read = read_context::ReadContext::capture(state, false).await?;
    let before = read.status().await?;
    let options = crate::session::query(&read.client, "GetPlanEditorOptions", &input).await?;
    let _guard = read.finish(state, &before).await?;
    Ok(options)
}
pub async fn preview_rename(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    input: RenameInput,
) -> Result<MutationOutcome, DesktopFailure> {
    main_window(window)?;
    let epoch = state.2.load(Ordering::SeqCst);
    let action = ActionDiagnostics::begin(state.3, ActionOperation::Rename);
    action.preview(PreviewPhase::Begin, None);
    let context = {
        let mut guard = confirmation_session(state, epoch).await?;
        guard
            .as_mut()
            .ok_or("RESIDENT_UNAVAILABLE")?
            .preview(input)
            .await?
    };
    action.preview(PreviewPhase::End, Some(context.revision()));
    confirm(
        window,
        state,
        ActionOperation::Rename,
        NativeConfirmation::Rename(Box::new(context)),
        epoch,
    )
    .await?
    .mutation()
}
pub async fn preview_restore_name(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    input: RestoreInput,
) -> Result<MutationOutcome, DesktopFailure> {
    main_window(window)?;
    let epoch = state.2.load(Ordering::SeqCst);
    let action = ActionDiagnostics::begin(state.3, ActionOperation::RestoreName);
    action.preview(PreviewPhase::Begin, None);
    let context = confirmation_session(state, epoch)
        .await?
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .preview_restore(input)
        .await?;
    action.preview(PreviewPhase::End, Some(context.revision()));
    confirm(
        window,
        state,
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
    SubscriptionCheck(Box<hiroute_application_api::ComputeSubscriptionCheckResultV2>),
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
pub async fn preview_price_change(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    input: PriceEditInput,
) -> Result<MutationOutcome, DesktopFailure> {
    main_window(window)?;
    let epoch = state.2.load(Ordering::SeqCst);
    let action = ActionDiagnostics::begin(state.3, ActionOperation::PriceChange);
    action.preview(PreviewPhase::Begin, None);
    let context = confirmation_session(state, epoch)
        .await?
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .preview_price(input)
        .await?;
    action.preview(PreviewPhase::End, Some(context.revision()));
    confirm(
        window,
        state,
        ActionOperation::PriceChange,
        NativeConfirmation::Price(Box::new(context)),
        epoch,
    )
    .await?
    .mutation()
}
pub async fn model_reference_query(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    input: ModelCatalogQueryV1,
) -> Result<ModelCatalogResultV1, DesktopFailure> {
    main_window(window)?;
    let guard = state.0.lock().await;
    let session = guard.as_ref().ok_or("RESIDENT_UNAVAILABLE")?;
    crate::session::query(&session.client, "ShowModel", &input).await
}
pub async fn effective_price_query(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    input: GetEffectivePricesV2,
) -> Result<PriceDisplayResult, DesktopFailure> {
    main_window(window)?;
    let guard = state.0.lock().await;
    let session = guard.as_ref().ok_or("RESIDENT_UNAVAILABLE")?;
    Ok(PriceDisplayResult::new(
        crate::session::query(&session.client, "GetEffectivePrices", &input).await?,
    ))
}

mod agent_snapshot;
pub use agent_snapshot::agent_snapshot;
pub async fn retry_agent_settings(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    input: hiroute_application_api::AgentSettingsRetryV1,
) -> Result<Value, DesktopFailure> {
    main_window(window)?;
    state
        .0
        .lock()
        .await
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .retry_agent_settings(input)
        .await
}
pub async fn preview_agent_settings(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    input: AgentSettingsInput,
) -> Result<AgentOutcome, DesktopFailure> {
    main_window(window)?;
    let epoch = state.2.load(Ordering::SeqCst);
    let action = ActionDiagnostics::begin(state.3, ActionOperation::AgentSettings);
    action.preview(PreviewPhase::Begin, None);
    let prepared = confirmation_session(state, epoch)
        .await?
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
                window,
                state,
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
pub async fn check_agent_authentication(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    input: AgentCheckInput,
) -> Result<bool, DesktopFailure> {
    main_window(window)?;
    let epoch = state.2.load(Ordering::SeqCst);
    let action = ActionDiagnostics::begin(state.3, ActionOperation::AgentCheck);
    action.preview(PreviewPhase::Begin, None);
    let context = confirmation_session(state, epoch)
        .await?
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .prepare_agent_check(input)
        .await?;
    action.preview(PreviewPhase::End, Some(context.revision()));
    match confirm(
        window,
        state,
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

pub async fn check_agent_live(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    input: AgentCheckInput,
) -> Result<AgentCheckCompletion, DesktopFailure> {
    main_window(window)?;
    if input.scope.as_deref() != Some("live") {
        return Err("AGENT_INPUT_INVALID".into());
    }
    let epoch = state.2.load(Ordering::SeqCst);
    let action = ActionDiagnostics::begin(state.3, ActionOperation::AgentCheck);
    action.preview(PreviewPhase::Begin, None);
    let context = confirmation_session(state, epoch)
        .await?
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .prepare_agent_check(input)
        .await?;
    action.preview(PreviewPhase::End, Some(context.revision()));
    match confirm(
        window,
        state,
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
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    operation: ActionOperation,
    context: NativeConfirmation,
    epoch: u64,
) -> Result<NativeOutcome, DesktopFailure> {
    let action = ActionDiagnostics::begin(state.3, operation);
    if state.2.load(Ordering::SeqCst) != epoch || !window.is_visible().unwrap_or(false) {
        let mut guard = state.0.lock().await;
        // Recovery may have installed another Session while this callback waited.
        if state.2.load(Ordering::SeqCst) == epoch {
            guard
                .as_mut()
                .ok_or("RESIDENT_UNAVAILABLE")?
                .invalidate_confirmation();
        }
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
        let mut guard = state.0.lock().await;
        if state.2.load(Ordering::SeqCst) == epoch {
            guard
                .as_mut()
                .ok_or("RESIDENT_UNAVAILABLE")?
                .invalidate_confirmation();
        }
        action.confirmation(ConfirmationPhase::Expired, None);
        action.end(ActionResult::Stale, None);
        return Err("CONFIRMATION_STALE".into());
    }
    action.apply(ApplyPhase::Begin, Some(revision), None);
    let outcome = if let NativeConfirmation::SubscriptionCheck(context) = context {
        let pending = {
            let mut guard = confirmation_session(state, epoch).await?;
            guard
                .as_mut()
                .ok_or("RESIDENT_UNAVAILABLE")?
                .begin_subscription_check_confirmation(*context, accepted)
                .await?
        };
        // Operation A performs connector I/O. Keep native close/recovery commands responsive while
        // it runs so close can submit cancellation rather than waiting behind this command.
        let completion = pending.execute().await;
        // Completion belongs to the Session that submitted it, not the window's prompt epoch.
        // Window close must still release an A resource created after the close-time sweep.
        crate::session::complete_subscription_check(state.0, completion)
            .await
            .map(Box::new)
            .map(NativeOutcome::SubscriptionCheck)
    } else if let NativeConfirmation::Check(context) = context {
        let dispatch = {
            let mut guard = confirmation_session(state, epoch).await?;
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
        let mut guard = confirmation_session(state, epoch).await?;
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
pub async fn observe_operation(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
) -> Result<Option<ClientOperationViewV1>, DesktopFailure> {
    main_window(window)?;
    state
        .0
        .lock()
        .await
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .observe()
        .await
}
pub async fn stop_observing(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
) -> Result<(), String> {
    main_window(window)?;
    state
        .0
        .lock()
        .await
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .stop_observing();
    Ok(())
}

mod dispatch;
pub use dispatch::dispatch;

#[cfg(test)]
mod action_result_tests;
