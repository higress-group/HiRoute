use super::{DesktopState, main_window};
use crate::failure::DesktopFailure;
use hiroute_application_api::{
    DelegationAcceptedV1, DelegationCancelV1, DelegationGetV1, DelegationListV1,
    DelegationResultV1, DelegationWaitV1, MachineEnvelopeV2, WorkPlanListV1, WorkerCancelRequestV1,
    WorkerContinueRequestV1, WorkerDependenciesDiscoverRequestV1,
    WorkerDependenciesSelectRequestV1, WorkerDependenciesViewV1, WorkerExecutorAvailabilityListV1,
    WorkerListRequestV1, WorkerPlansRequestV1, WorkerReadDataV1, WorkerReadRequestV1,
    WorkerResultRequestV1, WorkerSettingsV1, WorkerStatusRequestV1, WorkerWaitRequestV1,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{State, WebviewWindow};

const WORKER_DEPENDENCY_CONFIRMATION_SCHEMA_V1: &str =
    "hiroute.worker-dependency-selection-confirmation/v1";
const WORKER_DEPENDENCY_CONFIRMATION_TTL_MS: u64 = 60_000;

#[derive(Clone, Debug, Serialize)]
pub(super) struct WorkerDependencyConfirmationV1 {
    schema: &'static str,
    confirmation_id: String,
    selection: WorkerDependenciesSelectRequestV1,
    expires_at_ms: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WorkerDependencyConfirmationInput {
    confirmation_id: String,
}

#[derive(Clone)]
struct PendingWorkerDependencyConfirmation {
    window_label: String,
    selection: WorkerDependenciesSelectRequestV1,
    expires_at_ms: u64,
}

#[derive(Default)]
pub(super) struct WorkerDependencyConfirmationState {
    pending: Mutex<BTreeMap<String, PendingWorkerDependencyConfirmation>>,
}

impl WorkerDependencyConfirmationState {
    fn lock(
        &self,
    ) -> std::sync::MutexGuard<'_, BTreeMap<String, PendingWorkerDependencyConfirmation>> {
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn prepare(
        &self,
        window_label: &str,
        selection: WorkerDependenciesSelectRequestV1,
        now_ms: u64,
    ) -> Result<WorkerDependencyConfirmationV1, DesktopFailure> {
        let expires_at_ms = now_ms
            .checked_add(WORKER_DEPENDENCY_CONFIRMATION_TTL_MS)
            .ok_or("CLOCK_UNAVAILABLE")?;
        let confirmation_id = format!("worker-dependency-confirmation/{}", crate::random_id()?);
        let mut pending = self.lock();
        pending.retain(|_, entry| entry.window_label != window_label);
        pending.insert(
            confirmation_id.clone(),
            PendingWorkerDependencyConfirmation {
                window_label: window_label.to_owned(),
                selection: selection.clone(),
                expires_at_ms,
            },
        );
        Ok(WorkerDependencyConfirmationV1 {
            schema: WORKER_DEPENDENCY_CONFIRMATION_SCHEMA_V1,
            confirmation_id,
            selection,
            expires_at_ms,
        })
    }

    fn take(
        &self,
        window_label: &str,
        confirmation_id: &str,
        now_ms: u64,
    ) -> Result<WorkerDependenciesSelectRequestV1, DesktopFailure> {
        if !valid_worker_dependency_confirmation_id(confirmation_id) {
            return Err("CONFIRMATION_STALE".into());
        }
        let entry = {
            let mut pending = self.lock();
            if pending
                .get(confirmation_id)
                .is_none_or(|entry| entry.window_label != window_label)
            {
                return Err("CONFIRMATION_STALE".into());
            }
            pending
                .remove(confirmation_id)
                .expect("matching Worker dependency confirmation exists")
        };
        if now_ms > entry.expires_at_ms {
            return Err("CONFIRMATION_EXPIRED".into());
        }
        Ok(entry.selection)
    }

    fn cancel(&self, window_label: &str, confirmation_id: &str) -> Result<(), DesktopFailure> {
        if !valid_worker_dependency_confirmation_id(confirmation_id) {
            return Err("CONFIRMATION_STALE".into());
        }
        let mut pending = self.lock();
        if pending
            .get(confirmation_id)
            .is_some_and(|entry| entry.window_label != window_label)
        {
            return Err("CONFIRMATION_STALE".into());
        }
        pending.remove(confirmation_id);
        Ok(())
    }

    pub(super) fn cancel_window(&self, window_label: &str) {
        self.lock()
            .retain(|_, entry| entry.window_label != window_label);
    }
}

fn valid_worker_dependency_confirmation_id(value: &str) -> bool {
    value
        .strip_prefix("worker-dependency-confirmation/")
        .is_some_and(|id| {
            id.len() == 64
                && id
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
}

fn now_ms() -> Result<u64, DesktopFailure> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "CLOCK_UNAVAILABLE")?
        .as_millis()
        .try_into()
        .map_err(|_| "CLOCK_UNAVAILABLE")?)
}

fn normalize_worker_dependency_selection(
    request: &WorkerDependenciesSelectRequestV1,
) -> Result<WorkerDependenciesSelectRequestV1, DesktopFailure> {
    if !request.valid() {
        return Err("REQUEST_INVALID".into());
    }
    // The shared launch form requires adapters for Codex/Claude and forbids them for
    // native ACP clients. Only normalize components that the validated shape contains.
    let adapter = request
        .adapter_path
        .as_deref()
        .map(|path| normalize_required_path(Path::new(path), request.node_path.is_none()))
        .transpose()?;
    let cli = normalize_required_path(Path::new(&request.cli_path), true)?;
    let node = request
        .node_path
        .as_deref()
        .map(|path| normalize_required_path(Path::new(path), true))
        .transpose()?;
    let normalized = WorkerDependenciesSelectRequestV1 {
        harness: request.harness,
        adapter_path: adapter.map(normalized_path_string).transpose()?,
        cli_path: normalized_path_string(cli)?,
        node_path: node.map(normalized_path_string).transpose()?,
        expected_selection_revision: request.expected_selection_revision,
    };
    hiroute_application_api::plan_worker_dependency_selection(&normalized)
        .ok_or("REQUEST_INVALID")?;
    Ok(normalized)
}

fn normalize_required_path(
    path: &Path,
    require_executable: bool,
) -> Result<PathBuf, DesktopFailure> {
    let canonical = match fs::canonicalize(path) {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err("WORKER_DEPENDENCIES_MISSING".into());
        }
        Err(_) => return Err("WORKER_DEPENDENCIES_UNAVAILABLE".into()),
    };
    let metadata = fs::metadata(&canonical).map_err(|_| "WORKER_DEPENDENCIES_UNAVAILABLE")?;
    if !metadata.is_file()
        || (require_executable && !path_is_executable(&metadata))
        || (!require_executable && !path_is_readable(&metadata))
    {
        return Err("WORKER_DEPENDENCIES_INVALID".into());
    }
    Ok(canonical)
}

fn normalized_path_string(path: PathBuf) -> Result<String, DesktopFailure> {
    path.into_os_string()
        .into_string()
        .ok()
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 4096
                && !value.contains(char::is_control)
                && Path::new(value).is_absolute()
        })
        .ok_or_else(|| "WORKER_DEPENDENCIES_INVALID".into())
}

fn path_is_executable(metadata: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        metadata.is_file()
    }
}

fn path_is_readable(metadata: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o444 != 0
    }
    #[cfg(not(unix))]
    {
        metadata.is_file()
    }
}

fn data<R>(envelope: MachineEnvelopeV2<R>) -> Result<R, DesktopFailure>
where
    R: DeserializeOwned + Serialize,
{
    if envelope.error.is_some() {
        let encoded = serde_json::to_value(envelope).map_err(|_| "RESPONSE_DATA_INVALID")?;
        let untyped = serde_json::from_value(encoded).map_err(|_| "RESPONSE_DATA_INVALID")?;
        return Err(DesktopFailure::backend(untyped));
    }
    envelope
        .data
        .ok_or_else(|| DesktopFailure::from("RESPONSE_DATA_MISSING"))
}

#[tauri::command]
pub async fn worker_settings_get(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<WorkerSettingsV1, DesktopFailure> {
    main_window(&window)?;
    let client = state
        .0
        .lock()
        .await
        .as_ref()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .client
        .clone();
    let result = data(client.worker_settings(&crate::random_id()?).await?)?;
    if !result.valid() {
        return Err("RESPONSE_DATA_INVALID".into());
    }
    Ok(result)
}

#[tauri::command]
pub async fn worker_settings_set(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: WorkerSettingsV1,
) -> Result<WorkerSettingsV1, DesktopFailure> {
    main_window(&window)?;
    if !input.valid() {
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
    let result = data(
        client
            .set_worker_settings(&crate::random_id()?, &input)
            .await?,
    )?;
    if !result.valid() {
        return Err("RESPONSE_DATA_INVALID".into());
    }
    Ok(result)
}

#[tauri::command]
pub async fn worker_executor_availability(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<WorkerExecutorAvailabilityListV1, DesktopFailure> {
    main_window(&window)?;
    let client = state
        .0
        .lock()
        .await
        .as_ref()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .client
        .clone();
    let result = data(
        client
            .worker_executor_availability(&crate::random_id()?)
            .await?,
    )?;
    if !result.valid() {
        return Err("RESPONSE_DATA_INVALID".into());
    }
    Ok(result)
}

#[tauri::command]
pub async fn worker_task_plans(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: WorkerPlansRequestV1,
) -> Result<MachineEnvelopeV2<WorkPlanListV1>, DesktopFailure> {
    main_window(&window)?;
    if !input.valid() {
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
    Ok(client.worker_plans(&crate::random_id()?, &input).await?)
}

#[tauri::command]
pub async fn worker_dependencies_discover(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: WorkerDependenciesDiscoverRequestV1,
) -> Result<MachineEnvelopeV2<WorkerDependenciesViewV1>, DesktopFailure> {
    main_window(&window)?;
    if !input.valid() {
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
    Ok(client
        .worker_dependencies_discover(&crate::random_id()?, &input)
        .await?)
}

#[tauri::command]
pub async fn worker_dependencies_select_prepare(
    window: WebviewWindow,
    state: State<'_, WorkerDependencyConfirmationState>,
    input: WorkerDependenciesSelectRequestV1,
) -> Result<WorkerDependencyConfirmationV1, DesktopFailure> {
    main_window(&window)?;
    let selection = normalize_worker_dependency_selection(&input)?;
    state.prepare(window.label(), selection, now_ms()?)
}

#[tauri::command]
pub async fn worker_dependencies_select_confirm(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    confirmations: State<'_, WorkerDependencyConfirmationState>,
    input: WorkerDependencyConfirmationInput,
) -> Result<MachineEnvelopeV2<WorkerDependenciesViewV1>, DesktopFailure> {
    main_window(&window)?;
    let selection = confirmations.take(window.label(), &input.confirmation_id, now_ms()?)?;
    state
        .0
        .lock()
        .await
        .as_mut()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .select_worker_dependencies(selection)
        .await
}

#[tauri::command]
pub async fn worker_dependencies_select_cancel(
    window: WebviewWindow,
    state: State<'_, WorkerDependencyConfirmationState>,
    input: WorkerDependencyConfirmationInput,
) -> Result<(), DesktopFailure> {
    main_window(&window)?;
    state.cancel(window.label(), &input.confirmation_id)
}

#[tauri::command]
pub async fn worker_task_list(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: WorkerListRequestV1,
) -> Result<MachineEnvelopeV2<DelegationListV1>, DesktopFailure> {
    main_window(&window)?;
    if !input.valid() {
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
    Ok(client.worker_list(&crate::random_id()?, &input).await?)
}

#[tauri::command]
pub async fn worker_task_status(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: WorkerStatusRequestV1,
) -> Result<MachineEnvelopeV2<DelegationGetV1>, DesktopFailure> {
    main_window(&window)?;
    if !input.valid() {
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
    Ok(client.worker_status(&crate::random_id()?, &input).await?)
}

#[tauri::command]
pub async fn worker_task_result(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: WorkerResultRequestV1,
) -> Result<MachineEnvelopeV2<DelegationResultV1>, DesktopFailure> {
    main_window(&window)?;
    if !input.valid() {
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
    Ok(client.worker_result(&crate::random_id()?, &input).await?)
}

#[tauri::command]
pub async fn worker_task_read(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: WorkerReadRequestV1,
) -> Result<MachineEnvelopeV2<WorkerReadDataV1>, DesktopFailure> {
    main_window(&window)?;
    if !input.valid() {
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
    Ok(client.worker_read(&crate::random_id()?, &input).await?)
}

#[tauri::command]
pub async fn worker_task_wait(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: WorkerWaitRequestV1,
) -> Result<MachineEnvelopeV2<DelegationWaitV1>, DesktopFailure> {
    main_window(&window)?;
    if !input.valid() {
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
    Ok(client.worker_wait(&crate::random_id()?, &input).await?)
}

#[tauri::command]
pub async fn worker_task_cancel(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: WorkerCancelRequestV1,
) -> Result<MachineEnvelopeV2<DelegationCancelV1>, DesktopFailure> {
    main_window(&window)?;
    if !input.valid() {
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
    Ok(client.worker_cancel(&crate::random_id()?, &input).await?)
}

#[tauri::command]
pub async fn worker_task_continue(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    input: WorkerContinueRequestV1,
) -> Result<MachineEnvelopeV2<DelegationAcceptedV1>, DesktopFailure> {
    main_window(&window)?;
    if !input.valid() {
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
    Ok(client.worker_continue(&crate::random_id()?, &input).await?)
}

#[cfg(test)]
#[path = "worker_dependencies_tests.rs"]
mod tests;
