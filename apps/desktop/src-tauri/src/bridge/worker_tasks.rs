use super::*;
#[tauri::command]
pub async fn worker_settings_get(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<WorkerSettingsV1, DesktopFailure> {
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
    hiroute_product_session::commands::worker_settings_get(&context, &product_state).await
}
#[tauri::command]
pub async fn worker_settings_set(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: WorkerSettingsV1,
) -> Result<WorkerSettingsV1, DesktopFailure> {
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
    hiroute_product_session::commands::worker_settings_set(&context, &product_state, input).await
}
#[tauri::command]
pub async fn worker_executor_availability(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<WorkerExecutorAvailabilityListV1, DesktopFailure> {
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
    hiroute_product_session::commands::worker_executor_availability(&context, &product_state).await
}
#[tauri::command]
pub async fn worker_task_plans(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: WorkerPlansRequestV1,
) -> Result<MachineEnvelopeV2<WorkPlanListV1>, DesktopFailure> {
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
    hiroute_product_session::commands::worker_task_plans(&context, &product_state, input).await
}
#[tauri::command]
pub async fn worker_dependencies_discover(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: WorkerDependenciesDiscoverRequestV1,
) -> Result<MachineEnvelopeV2<WorkerDependenciesViewV1>, DesktopFailure> {
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
    hiroute_product_session::commands::worker_dependencies_discover(&context, &product_state, input)
        .await
}
#[tauri::command]
pub async fn worker_dependencies_select_prepare(
    window: WebviewWindow,
    state: State<'_, WorkerDependencyConfirmationState>,
    desktop: State<'_, DesktopState>,
    input: WorkerDependenciesSelectRequestV1,
) -> Result<WorkerDependencyConfirmationV1, DesktopFailure> {
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
        &desktop.0,
        &desktop.1.error,
        &desktop.2,
        &desktop.3,
    );
    hiroute_product_session::commands::worker_dependencies_select_prepare(
        &context,
        &state,
        &product_state,
        input,
    )
    .await
}
#[tauri::command]
pub async fn worker_dependencies_select_confirm(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    confirmations: State<'_, WorkerDependencyConfirmationState>,
    input: WorkerDependencyConfirmationInput,
) -> Result<MachineEnvelopeV2<WorkerDependenciesViewV1>, DesktopFailure> {
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
    hiroute_product_session::commands::worker_dependencies_select_confirm(
        &context,
        &product_state,
        &confirmations,
        input,
    )
    .await
}
#[tauri::command]
pub async fn worker_dependencies_select_cancel(
    window: WebviewWindow,
    state: State<'_, WorkerDependencyConfirmationState>,
    input: WorkerDependencyConfirmationInput,
) -> Result<(), DesktopFailure> {
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
    hiroute_product_session::commands::worker_dependencies_select_cancel(&context, &state, input)
        .await
}
#[tauri::command]
pub async fn worker_task_list(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: WorkerListRequestV1,
) -> Result<MachineEnvelopeV2<DelegationListV1>, DesktopFailure> {
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
    hiroute_product_session::commands::worker_task_list(&context, &product_state, input).await
}
#[tauri::command]
pub async fn worker_task_status(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: WorkerStatusRequestV1,
) -> Result<MachineEnvelopeV2<DelegationGetV1>, DesktopFailure> {
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
    hiroute_product_session::commands::worker_task_status(&context, &product_state, input).await
}
#[tauri::command]
pub async fn worker_task_result(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: WorkerResultRequestV1,
) -> Result<MachineEnvelopeV2<DelegationResultV1>, DesktopFailure> {
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
    hiroute_product_session::commands::worker_task_result(&context, &product_state, input).await
}
#[tauri::command]
pub async fn worker_task_read(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: WorkerReadRequestV1,
) -> Result<MachineEnvelopeV2<WorkerReadDataV1>, DesktopFailure> {
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
    hiroute_product_session::commands::worker_task_read(&context, &product_state, input).await
}
#[tauri::command]
pub async fn worker_task_wait(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: WorkerWaitRequestV1,
) -> Result<MachineEnvelopeV2<DelegationWaitV1>, DesktopFailure> {
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
    hiroute_product_session::commands::worker_task_wait(&context, &product_state, input).await
}
#[tauri::command]
pub async fn worker_task_confirm_residual(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: WorkerResidualConfirmRequestV1,
) -> Result<MachineEnvelopeV2<DelegationRunViewV1>, DesktopFailure> {
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
    hiroute_product_session::commands::worker_task_confirm_residual(&context, &product_state, input)
        .await
}
#[tauri::command]
pub async fn worker_task_cancel(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: WorkerCancelRequestV1,
) -> Result<MachineEnvelopeV2<DelegationCancelV1>, DesktopFailure> {
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
    hiroute_product_session::commands::worker_task_cancel(&context, &product_state, input).await
}
#[tauri::command]
pub async fn worker_task_continue(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: WorkerContinueRequestV1,
) -> Result<MachineEnvelopeV2<DelegationAcceptedV1>, DesktopFailure> {
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
    hiroute_product_session::commands::worker_task_continue(&context, &product_state, input).await
}
