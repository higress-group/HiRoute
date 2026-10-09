//! Typed delegation task admission at the common Local Control boundary.
//!
//! Public Worker operations trust the local same-UID channel and use the daemon instance as their
//! task namespace. The daemon-owned port performs exact-Plan admission and schedules an already
//! accepted run. Retired protected task operations fail closed and are not a compatibility path.
use crate::{ApplicationService, failed, succeeded};
use hiroute_application_api::{
    DelegationAcceptedV1, DelegationCancelRequestV1, DelegationCancelV1,
    DelegationContinueRequestV1, DelegationGetRequestV1, DelegationGetV1, DelegationListRequestV1,
    DelegationListV1, DelegationResultRequestV1, DelegationResultV1, DelegationRunViewV1,
    DelegationStartRequestV1, DelegationTaskViewV1, DelegationWaitRequestV1, DelegationWaitV1,
    ErrorCode, LocalControlRequestV2, MachineEnvelopeV2, WORKER_EXECUTOR_AVAILABILITY_OPERATION_V1,
    WORKER_SETTINGS_GET_OPERATION_V1, WORKER_SETTINGS_SET_OPERATION_V1, WorkPlanListV1,
    WorkerActionFactsV1, WorkerCancelRequestV1, WorkerContinueRequestV1,
    WorkerDependenciesDiscoverRequestV1, WorkerDependenciesSelectRequestV1,
    WorkerDependenciesViewV1, WorkerExecRequestV1, WorkerExecutorAvailabilityListV1,
    WorkerListRequestV1, WorkerObservedOperationV1, WorkerPlansRequestV1, WorkerReadActionStateV1,
    WorkerReadContentStateV1, WorkerReadDataV1, WorkerReadRequestV1,
    WorkerResidualConfirmRequestV1, WorkerResultRequestV1, WorkerRunActionFactsV1,
    WorkerSettingsV1, WorkerStatusRequestV1, WorkerWaitRequestV1, normalize_worker_title,
    worker_next_actions,
};
use hiroute_domain::VerifiedCollaborationPrincipal;
use hiroute_domain::delegation::DelegationErrorV1;
use serde::{Serialize, de::DeserializeOwned};
use std::sync::Arc;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkerReadErrorV1 {
    Delegation(DelegationErrorV1),
    InvalidCursor,
    PageTooSmall,
    CursorStale,
    CursorEvicted { recovery_cursor: String },
    CursorGap { recovery_cursor: String },
    Unavailable,
    StorageUnavailable,
}

impl From<DelegationErrorV1> for WorkerReadErrorV1 {
    fn from(error: DelegationErrorV1) -> Self {
        Self::Delegation(error)
    }
}

pub trait DelegationTaskPort: Send + Sync {
    fn worker_settings(&self) -> Result<WorkerSettingsV1, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }
    fn set_worker_settings(
        &self,
        _settings: &WorkerSettingsV1,
    ) -> Result<WorkerSettingsV1, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }
    fn worker_executor_availability(
        &self,
    ) -> Result<WorkerExecutorAvailabilityListV1, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }
    fn worker_plans(
        &self,
        _request: &WorkerPlansRequestV1,
    ) -> Result<WorkPlanListV1, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }
    fn worker_dependencies_discover(
        &self,
        _request: &WorkerDependenciesDiscoverRequestV1,
    ) -> Result<WorkerDependenciesViewV1, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }
    /// Performs only bounded native metadata normalization and validation. Application retains
    /// ownership of the protected digest, typed plan, CAS and Operation admission.
    fn validate_worker_dependency_selection(
        &self,
        _request: &WorkerDependenciesSelectRequestV1,
    ) -> Result<WorkerDependenciesSelectRequestV1, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }
    fn worker_exec(
        &self,
        _request: &WorkerExecRequestV1,
    ) -> Result<DelegationAcceptedV1, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }
    fn worker_list(
        &self,
        _request: &WorkerListRequestV1,
    ) -> Result<DelegationListV1, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }
    fn worker_status(
        &self,
        _request: &WorkerStatusRequestV1,
    ) -> Result<DelegationGetV1, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }
    fn worker_wait(
        &self,
        _request: &WorkerWaitRequestV1,
    ) -> Result<DelegationWaitV1, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }
    fn worker_result(
        &self,
        _request: &WorkerResultRequestV1,
    ) -> Result<DelegationResultV1, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }
    fn worker_read(
        &self,
        _request: &WorkerReadRequestV1,
    ) -> Result<WorkerReadDataV1, WorkerReadErrorV1> {
        Err(WorkerReadErrorV1::Unavailable)
    }
    fn worker_cancel(
        &self,
        _request: &WorkerCancelRequestV1,
    ) -> Result<DelegationCancelV1, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }
    fn confirm_worker_residual(
        &self,
        _request: &WorkerResidualConfirmRequestV1,
    ) -> Result<DelegationRunViewV1, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }
    fn worker_continue(
        &self,
        _request: &WorkerContinueRequestV1,
    ) -> Result<DelegationAcceptedV1, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }
    fn list(
        &self,
        _principal: &VerifiedCollaborationPrincipal,
        _request: &DelegationListRequestV1,
    ) -> Result<DelegationListV1, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }
    fn get(
        &self,
        _principal: &VerifiedCollaborationPrincipal,
        _request: &DelegationGetRequestV1,
    ) -> Result<DelegationGetV1, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }
    fn wait(
        &self,
        _principal: &VerifiedCollaborationPrincipal,
        _request: &DelegationWaitRequestV1,
    ) -> Result<DelegationWaitV1, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }
    fn result(
        &self,
        _principal: &VerifiedCollaborationPrincipal,
        _request: &DelegationResultRequestV1,
    ) -> Result<DelegationResultV1, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }
    fn cancel(
        &self,
        _principal: &VerifiedCollaborationPrincipal,
        _request: &DelegationCancelRequestV1,
    ) -> Result<DelegationCancelV1, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }
    fn continue_task(
        &self,
        _principal: &VerifiedCollaborationPrincipal,
        _request: &DelegationContinueRequestV1,
    ) -> Result<DelegationAcceptedV1, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }
    /// Commits the sole durable acceptance record before arranging asynchronous execution.  A
    /// replay with the same owner/key/request digest returns the original run and never causes a
    /// prompt to be re-sent.
    fn start(
        &self,
        _principal: &VerifiedCollaborationPrincipal,
        _request: &DelegationStartRequestV1,
    ) -> Result<DelegationAcceptedV1, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }
}

pub struct DelegationTasks {
    port: Arc<dyn DelegationTaskPort>,
}

impl DelegationTasks {
    pub fn new(port: Arc<dyn DelegationTaskPort>) -> Self {
        Self { port }
    }
}

pub(crate) fn dispatch(
    service: &ApplicationService,
    request: LocalControlRequestV2,
) -> MachineEnvelopeV2<serde_json::Value> {
    if request.operation_id == hiroute_application_api::WORKER_DEPENDENCIES_SELECT_OPERATION_V1 {
        return dispatch_worker_dependency_selection(service, request);
    }
    if request.operation_id.starts_with("Worker") {
        return dispatch_worker(service, request);
    }
    match request.operation_id.as_str() {
        // The MVP has one current instance-scoped Worker producer. Protected task operations
        // remain recognized only long enough to fail closed; they are not a compatibility path.
        "ListDelegations"
        | "GetDelegation"
        | "WaitDelegation"
        | "ReadDelegationResult"
        | "CancelDelegation"
        | "ContinueDelegation"
        | "StartDelegation" => failed(ErrorCode::CapabilityUnavailable, request.request_id),
        _ => failed(ErrorCode::UnknownCommand, request.request_id),
    }
}

fn dispatch_worker_dependency_selection(
    service: &ApplicationService,
    request: LocalControlRequestV2,
) -> MachineEnvelopeV2<serde_json::Value> {
    if request.protected_grant.is_some() {
        return failed(ErrorCode::CapabilityDenied, request.request_id);
    }
    let Ok(input) = serde_json::from_value::<WorkerDependenciesSelectRequestV1>(request.payload)
    else {
        return failed(ErrorCode::InvalidArguments, request.request_id);
    };
    let Some(initial_plan) = hiroute_application_api::plan_worker_dependency_selection(&input)
    else {
        return failed(ErrorCode::InvalidArguments, request.request_id);
    };
    let Some(ports) = service.ports.as_ref() else {
        return failed(ErrorCode::DaemonUnavailable, request.request_id);
    };
    let Some(tasks) = ports.delegation_tasks.as_ref() else {
        return failed(ErrorCode::CapabilityUnavailable, request.request_id);
    };
    let Some(mutation) = ports.mutation.as_ref() else {
        return failed(ErrorCode::DaemonUnavailable, request.request_id);
    };
    let workspace = hiroute_domain::WorkspaceId::default();
    match ports.control.operation_for_idempotency(
        &workspace,
        hiroute_application_api::PrincipalKind::InteractiveUser,
        hiroute_application_api::WORKER_DEPENDENCIES_SELECT_OPERATION_V1,
        &initial_plan.idempotency_key,
    ) {
        Ok(Some(operation)) if operation.accepted_digest == initial_plan.accept_digest => {
            return worker_dependency_selection_result(
                tasks,
                &input,
                operation,
                request.request_id,
            );
        }
        Ok(Some(_)) => return failed(ErrorCode::IdempotencyKeyReused, request.request_id),
        Err(_) => return failed(ErrorCode::DaemonUnavailable, request.request_id),
        Ok(None) => {}
    }
    let normalized = match tasks.port.validate_worker_dependency_selection(&input) {
        Ok(normalized) => normalized,
        Err(error) => return failed_delegation(error, request.request_id),
    };
    // The deterministic plan binds normalized paths. Silently replacing a path at the server
    // would change the accepted digest after the caller selected the exact tuple.
    if normalized != input {
        return failed(ErrorCode::InvalidArguments, request.request_id);
    }
    let Some(prepared_plan) =
        hiroute_application_api::plan_worker_dependency_selection(&normalized)
    else {
        return failed(ErrorCode::InvalidArguments, request.request_id);
    };
    let plan = match hiroute_domain::TransactionPlanV1::from_worker_dependency_selection_planner(
        prepared_plan.spec.clone(),
        prepared_plan.change,
    ) {
        Ok(plan) => plan,
        Err(_) => return failed(ErrorCode::InvalidArguments, request.request_id),
    };
    let apply = hiroute_application_api::ApplyRequestV1 {
        schema_version: hiroute_domain::CHANGE_SPEC_SCHEMA_V1,
        spec: prepared_plan.spec,
        accept_digest: prepared_plan.accept_digest.clone(),
        expected_revisions: prepared_plan.expected_revisions,
        idempotency_key: prepared_plan.idempotency_key,
        apply_capability: None,
    };
    let prepared = match crate::PreparedTransactionV1::for_worker_dependency_selection(
        apply,
        prepared_plan.accept_digest,
        plan,
    ) {
        Ok(prepared) => prepared,
        Err(error) => return failed(error.error_code(), request.request_id),
    };
    match mutation.apply_local_prepared_change(prepared) {
        Ok(operation) => {
            worker_dependency_selection_result(tasks, &normalized, operation, request.request_id)
        }
        Err(error) => failed(error.error_code(), request.request_id),
    }
}

fn worker_dependency_selection_result(
    tasks: &DelegationTasks,
    request: &WorkerDependenciesSelectRequestV1,
    operation: hiroute_domain::OperationV1,
    request_id: String,
) -> MachineEnvelopeV2<serde_json::Value> {
    let view = match tasks
        .port
        .worker_dependencies_discover(&WorkerDependenciesDiscoverRequestV1 {
            harness: Some(request.harness),
        }) {
        Ok(view) => view,
        Err(error) => return failed_delegation(error, request_id),
    };
    let reference = hiroute_application_api::OperationReferenceV1 {
        operation_id: operation.operation_id.to_string(),
        state: operation.state.as_str().to_owned(),
        sequence: operation.generation,
        cancellable: !operation.state.is_terminal(),
    };
    let mut envelope = MachineEnvelopeV2::accepted(
        serde_json::to_value(view).expect("Worker dependency view is serializable"),
        Some(request_id),
    );
    envelope.operation = Some(reference);
    envelope
}

fn dispatch_worker(
    service: &ApplicationService,
    request: LocalControlRequestV2,
) -> MachineEnvelopeV2<serde_json::Value> {
    if request.protected_grant.is_some() {
        return failed(ErrorCode::CapabilityDenied, request.request_id);
    }
    let Some(tasks) = service
        .ports
        .as_ref()
        .and_then(|ports| ports.delegation_tasks.as_ref())
    else {
        return failed(ErrorCode::CapabilityUnavailable, request.request_id);
    };
    match request.operation_id.as_str() {
        WORKER_SETTINGS_GET_OPERATION_V1 => {
            invoke_worker::<hiroute_application_api::ClientEmptyRequestV1, _>(
                tasks,
                request.payload,
                request.request_id,
                |_| true,
                |port, _| port.worker_settings(),
            )
        }
        WORKER_SETTINGS_SET_OPERATION_V1 => invoke_worker::<WorkerSettingsV1, _>(
            tasks,
            request.payload,
            request.request_id,
            WorkerSettingsV1::valid,
            |port, value| port.set_worker_settings(value),
        ),
        WORKER_EXECUTOR_AVAILABILITY_OPERATION_V1 => {
            invoke_worker::<hiroute_application_api::ClientEmptyRequestV1, _>(
                tasks,
                request.payload,
                request.request_id,
                |_| true,
                |port, _| port.worker_executor_availability(),
            )
        }
        "WorkerPlans" => invoke_worker::<WorkerPlansRequestV1, _>(
            tasks,
            request.payload,
            request.request_id,
            WorkerPlansRequestV1::valid,
            |port, value| port.worker_plans(value),
        ),
        hiroute_application_api::WORKER_DEPENDENCIES_DISCOVER_OPERATION_V1 => {
            invoke_worker::<WorkerDependenciesDiscoverRequestV1, _>(
                tasks,
                request.payload,
                request.request_id,
                WorkerDependenciesDiscoverRequestV1::valid,
                |port, value| port.worker_dependencies_discover(value),
            )
        }
        "WorkerExec" => invoke_worker_with_actions::<WorkerExecRequestV1, _, _, _>(
            tasks,
            request.payload,
            request.request_id,
            WorkerExecRequestV1::valid,
            |port, value| port.worker_exec(value),
            |_, accepted| accepted_actions(WorkerObservedOperationV1::Exec, accepted),
            |_, error| submission_error_actions(error),
        ),
        "WorkerList" => invoke_worker_with_actions::<WorkerListRequestV1, _, _, _>(
            tasks,
            request.payload,
            request.request_id,
            WorkerListRequestV1::valid,
            |port, value| port.worker_list(value),
            list_actions,
            |_, _| Vec::new(),
        ),
        "WorkerStatus" => invoke_worker_with_actions::<WorkerStatusRequestV1, _, _, _>(
            tasks,
            request.payload,
            request.request_id,
            WorkerStatusRequestV1::valid,
            |port, value| port.worker_status(value),
            |_, status| task_actions(WorkerObservedOperationV1::Status, &status.task),
            |_, _| Vec::new(),
        ),
        "WorkerWait" => invoke_worker_with_actions::<WorkerWaitRequestV1, _, _, _>(
            tasks,
            request.payload,
            request.request_id,
            WorkerWaitRequestV1::valid,
            |port, value| port.worker_wait(value),
            |_, wait| run_view_actions(WorkerObservedOperationV1::Wait, &wait.run, None, false),
            |_, _| Vec::new(),
        ),
        "WorkerResult" => invoke_worker_with_actions::<WorkerResultRequestV1, _, _, _>(
            tasks,
            request.payload,
            request.request_id,
            WorkerResultRequestV1::valid,
            |port, value| port.worker_result(value),
            |_, result| {
                run_view_actions(
                    WorkerObservedOperationV1::Result,
                    &result.run,
                    result.next_offset,
                    false,
                )
            },
            |_, _| Vec::new(),
        ),
        "WorkerRead" => invoke_worker_read(tasks, request.payload, request.request_id),
        "WorkerCancel" => invoke_worker_with_actions::<WorkerCancelRequestV1, _, _, _>(
            tasks,
            request.payload,
            request.request_id,
            WorkerCancelRequestV1::valid,
            |port, value| port.worker_cancel(value),
            |_, cancel| {
                run_view_actions(WorkerObservedOperationV1::Cancel, &cancel.run, None, false)
            },
            |_, _| Vec::new(),
        ),
        "WorkerConfirmResidual" => {
            invoke_worker_with_actions::<WorkerResidualConfirmRequestV1, _, _, _>(
                tasks,
                request.payload,
                request.request_id,
                WorkerResidualConfirmRequestV1::valid,
                |port, value| port.confirm_worker_residual(value),
                |_, run| run_view_actions(WorkerObservedOperationV1::Status, run, None, false),
                |_, _| Vec::new(),
            )
        }
        "WorkerContinue" => invoke_worker_with_actions::<WorkerContinueRequestV1, _, _, _>(
            tasks,
            request.payload,
            request.request_id,
            WorkerContinueRequestV1::valid,
            |port, value| port.worker_continue(value),
            |_, accepted| accepted_actions(WorkerObservedOperationV1::Continue, accepted),
            |_, error| submission_error_actions(error),
        ),
        _ => failed(ErrorCode::UnknownCommand, request.request_id),
    }
}

fn invoke_worker_with_actions<I, O, S, E>(
    tasks: &DelegationTasks,
    payload: serde_json::Value,
    request_id: String,
    valid: impl FnOnce(&I) -> bool,
    action: impl FnOnce(&dyn DelegationTaskPort, &I) -> Result<O, DelegationErrorV1>,
    success_actions: S,
    error_actions: E,
) -> MachineEnvelopeV2<serde_json::Value>
where
    I: DeserializeOwned,
    O: Serialize,
    S: FnOnce(&I, &O) -> Vec<hiroute_application_api::NextActionV1>,
    E: FnOnce(&I, DelegationErrorV1) -> Vec<hiroute_application_api::NextActionV1>,
{
    let Ok(input) = serde_json::from_value::<I>(payload) else {
        return failed(ErrorCode::InvalidArguments, request_id);
    };
    if !valid(&input) {
        return failed(ErrorCode::InvalidArguments, request_id);
    }
    match action(tasks.port.as_ref(), &input) {
        Ok(result) => {
            let actions = success_actions(&input, &result);
            let mut envelope = succeeded(result, request_id);
            envelope.next_actions = actions;
            envelope
        }
        Err(error) => {
            let actions = error_actions(&input, error);
            let mut envelope = failed_delegation(error, request_id);
            envelope.next_actions = actions;
            envelope
        }
    }
}

fn accepted_actions(
    operation: WorkerObservedOperationV1,
    accepted: &DelegationAcceptedV1,
) -> Vec<hiroute_application_api::NextActionV1> {
    worker_next_actions(&WorkerActionFactsV1::Run(WorkerRunActionFactsV1 {
        observed_operation: operation,
        task_id: accepted.task_id.clone(),
        run_id: accepted.run_id.clone(),
        run_state: accepted.state,
        state_revision: accepted.state_revision,
        read: WorkerReadActionStateV1::Unknown,
        result_available: false,
        result_next_offset: None,
        resume_available: false,
    }))
}

fn task_actions(
    operation: WorkerObservedOperationV1,
    task: &DelegationTaskViewV1,
) -> Vec<hiroute_application_api::NextActionV1> {
    run_view_actions(
        operation,
        &task.run,
        None,
        task.resumable_until_ms.is_some(),
    )
}

fn run_view_actions(
    operation: WorkerObservedOperationV1,
    run: &DelegationRunViewV1,
    result_next_offset: Option<u32>,
    resume_available: bool,
) -> Vec<hiroute_application_api::NextActionV1> {
    worker_next_actions(&WorkerActionFactsV1::Run(WorkerRunActionFactsV1 {
        observed_operation: operation,
        task_id: run.task_id.clone(),
        run_id: run.run_id.clone(),
        run_state: run.state,
        state_revision: run.state_revision,
        read: WorkerReadActionStateV1::Unknown,
        result_available: run.result_available,
        result_next_offset,
        resume_available,
    }))
}

fn read_actions(
    request: &WorkerReadRequestV1,
    read: &WorkerReadDataV1,
) -> Vec<hiroute_application_api::NextActionV1> {
    let read_state = match read.content_state {
        WorkerReadContentStateV1::Pending | WorkerReadContentStateV1::Available => {
            WorkerReadActionStateV1::Available {
                next_cursor: read
                    .next_cursor
                    .clone()
                    .expect("valid readable Worker data has a cursor"),
                max_bytes: request.max_bytes,
            }
        }
        WorkerReadContentStateV1::Deleted => WorkerReadActionStateV1::Deleted,
        WorkerReadContentStateV1::Expired => WorkerReadActionStateV1::Expired,
    };
    worker_next_actions(&WorkerActionFactsV1::Run(WorkerRunActionFactsV1 {
        observed_operation: WorkerObservedOperationV1::Read,
        task_id: read.task_id.clone(),
        run_id: read.run_id.clone(),
        run_state: read.run_state,
        state_revision: read.state_revision,
        read: read_state,
        result_available: false,
        result_next_offset: None,
        resume_available: false,
    }))
}

fn list_actions(
    request: &WorkerListRequestV1,
    page: &DelegationListV1,
) -> Vec<hiroute_application_api::NextActionV1> {
    let Some(cursor) = page.next_cursor.clone() else {
        return Vec::new();
    };
    let title = request
        .title
        .as_deref()
        .map(normalize_worker_title)
        .transpose()
        .expect("validated Worker title normalizes");
    worker_next_actions(&WorkerActionFactsV1::ListPage {
        title,
        cursor,
        limit: request.effective_limit(),
    })
}

fn submission_error_actions(
    error: DelegationErrorV1,
) -> Vec<hiroute_application_api::NextActionV1> {
    if matches!(
        error,
        DelegationErrorV1::CapabilityUnavailable | DelegationErrorV1::ResumeUnavailable
    ) {
        worker_next_actions(&WorkerActionFactsV1::PlanBlocked)
    } else {
        Vec::new()
    }
}

fn read_error_actions(
    request: &WorkerReadRequestV1,
    error: &WorkerReadErrorV1,
) -> Vec<hiroute_application_api::NextActionV1> {
    let read = match error {
        WorkerReadErrorV1::CursorEvicted { recovery_cursor } => {
            WorkerReadActionStateV1::CursorEvicted {
                recovery_cursor: recovery_cursor.clone(),
                max_bytes: request.max_bytes,
            }
        }
        WorkerReadErrorV1::CursorGap { recovery_cursor } => WorkerReadActionStateV1::CursorGap {
            recovery_cursor: recovery_cursor.clone(),
            max_bytes: request.max_bytes,
        },
        WorkerReadErrorV1::CursorStale => WorkerReadActionStateV1::CursorStale,
        WorkerReadErrorV1::Unavailable => WorkerReadActionStateV1::Unavailable,
        WorkerReadErrorV1::StorageUnavailable => WorkerReadActionStateV1::StorageUnavailable,
        WorkerReadErrorV1::Delegation(DelegationErrorV1::ContentUnavailable) => {
            WorkerReadActionStateV1::Unavailable
        }
        WorkerReadErrorV1::Delegation(DelegationErrorV1::StorageUnavailable) => {
            WorkerReadActionStateV1::StorageUnavailable
        }
        WorkerReadErrorV1::Delegation(_)
        | WorkerReadErrorV1::InvalidCursor
        | WorkerReadErrorV1::PageTooSmall => return Vec::new(),
    };
    worker_next_actions(&WorkerActionFactsV1::Run(WorkerRunActionFactsV1 {
        observed_operation: WorkerObservedOperationV1::Read,
        task_id: String::new(),
        run_id: request.run_id.clone(),
        run_state: hiroute_domain::delegation::RunStateV1::Unknown,
        state_revision: 0,
        read,
        result_available: false,
        result_next_offset: None,
        resume_available: false,
    }))
}

fn invoke_worker_read(
    tasks: &DelegationTasks,
    payload: serde_json::Value,
    request_id: String,
) -> MachineEnvelopeV2<serde_json::Value> {
    let Ok(input) = serde_json::from_value::<WorkerReadRequestV1>(payload) else {
        return failed(ErrorCode::InvalidArguments, request_id);
    };
    if !input.valid() {
        return failed(ErrorCode::InvalidArguments, request_id);
    }
    match tasks.port.worker_read(&input) {
        Ok(result) => {
            let actions = read_actions(&input, &result);
            let mut envelope = succeeded(result, request_id);
            envelope.next_actions = actions;
            envelope
        }
        Err(error) => {
            let actions = read_error_actions(&input, &error);
            let mut envelope = failed_worker_read(&error, request_id);
            envelope.next_actions = actions;
            envelope
        }
    }
}

fn failed_worker_read(
    error: &WorkerReadErrorV1,
    request_id: String,
) -> MachineEnvelopeV2<serde_json::Value> {
    let (code, message_key) = match error {
        WorkerReadErrorV1::InvalidCursor => {
            (ErrorCode::InvalidArguments, "worker.read.invalid_cursor")
        }
        WorkerReadErrorV1::PageTooSmall => {
            (ErrorCode::InvalidArguments, "worker.read.page_too_small")
        }
        WorkerReadErrorV1::CursorStale => (ErrorCode::RevisionConflict, "worker.read.cursor_stale"),
        WorkerReadErrorV1::CursorEvicted { .. } => {
            (ErrorCode::RevisionConflict, "worker.read.cursor_evicted")
        }
        WorkerReadErrorV1::CursorGap { .. } => {
            (ErrorCode::RevisionConflict, "worker.read.cursor_gap")
        }
        WorkerReadErrorV1::Unavailable => {
            (ErrorCode::ObservationUnavailable, "worker.read.unavailable")
        }
        WorkerReadErrorV1::StorageUnavailable => (
            ErrorCode::ObservationUnavailable,
            "worker.read.storage_unavailable",
        ),
        WorkerReadErrorV1::Delegation(DelegationErrorV1::ContentUnavailable) => {
            (ErrorCode::ObservationUnavailable, "worker.read.unavailable")
        }
        WorkerReadErrorV1::Delegation(DelegationErrorV1::StorageUnavailable) => (
            ErrorCode::ObservationUnavailable,
            "worker.read.storage_unavailable",
        ),
        WorkerReadErrorV1::Delegation(DelegationErrorV1::NotFound) => {
            (ErrorCode::ResourceNotFound, "worker.error.run_not_found")
        }
        WorkerReadErrorV1::Delegation(error) => {
            return failed_delegation(*error, request_id);
        }
    };
    let mut failure = hiroute_application_api::ErrorV1::new(code);
    failure.message_key = message_key.to_owned();
    MachineEnvelopeV2::failed(failure, Some(request_id))
}

fn invoke_worker<I, O>(
    tasks: &DelegationTasks,
    payload: serde_json::Value,
    request_id: String,
    valid: impl FnOnce(&I) -> bool,
    action: impl FnOnce(&dyn DelegationTaskPort, &I) -> Result<O, DelegationErrorV1>,
) -> MachineEnvelopeV2<serde_json::Value>
where
    I: DeserializeOwned,
    O: Serialize,
{
    let Ok(input) = serde_json::from_value::<I>(payload) else {
        return failed(ErrorCode::InvalidArguments, request_id);
    };
    if !valid(&input) {
        return failed(ErrorCode::InvalidArguments, request_id);
    }
    match action(tasks.port.as_ref(), &input) {
        Ok(result) => succeeded(result, request_id),
        Err(error) => failed_delegation(error, request_id),
    }
}

fn failed_delegation(
    error: DelegationErrorV1,
    request_id: String,
) -> MachineEnvelopeV2<serde_json::Value> {
    let mut failure = hiroute_application_api::ErrorV1::new(map_error(error));
    if let DelegationErrorV1::DependencyCheckFailed(reason) = error {
        failure.message_key = reason.message_key();
        failure.retryable = reason.reason.retryable();
        return MachineEnvelopeV2::failed(failure, Some(request_id));
    }
    failure.message_key = match error {
        DelegationErrorV1::DependenciesMissing => "worker.dependencies.missing",
        DelegationErrorV1::DependenciesInvalid => "worker.dependencies.invalid",
        DelegationErrorV1::DependenciesUnavailable => "worker.dependencies.unavailable",
        _ => return MachineEnvelopeV2::failed(failure, Some(request_id)),
    }
    .to_owned();
    MachineEnvelopeV2::failed(failure, Some(request_id))
}

fn map_error(error: DelegationErrorV1) -> ErrorCode {
    match error {
        DelegationErrorV1::InvalidArguments => ErrorCode::InvalidArguments,
        DelegationErrorV1::PermissionDenied => ErrorCode::CapabilityDenied,
        DelegationErrorV1::Conflict | DelegationErrorV1::Busy => ErrorCode::RevisionConflict,
        DelegationErrorV1::CapacityExceeded => ErrorCode::WorkerCapacityExceeded,
        DelegationErrorV1::NotFound => ErrorCode::ResourceNotFound,
        DelegationErrorV1::DeadlineExceeded => ErrorCode::ActionRequired,
        DelegationErrorV1::Cancelled
        | DelegationErrorV1::CapabilityUnavailable
        | DelegationErrorV1::DependenciesMissing
        | DelegationErrorV1::DependenciesInvalid
        | DelegationErrorV1::DependencyCheckFailed(_)
        | DelegationErrorV1::ResumeUnavailable
        | DelegationErrorV1::ContentUnavailable
        | DelegationErrorV1::StorageUnavailable
        | DelegationErrorV1::ProtocolFailed
        | DelegationErrorV1::PromptFailed => ErrorCode::CapabilityUnavailable,
        DelegationErrorV1::DependenciesUnavailable => ErrorCode::ObservationUnavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hiroute_application_api::MachineStatus;

    struct Tasks;
    impl DelegationTaskPort for Tasks {
        fn worker_executor_availability(
            &self,
        ) -> Result<WorkerExecutorAvailabilityListV1, DelegationErrorV1> {
            let unavailable = |harness| {
                hiroute_application_api::WorkerExecutorAvailabilityV1 {
                harness,
                state: hiroute_application_api::WorkerExecutorAvailabilityStateV1::Unavailable,
                reason: Some(
                    hiroute_application_api::WorkerExecutorAvailabilityReasonV1::InstallationNotConfigured,
                ),
                start_approve_all: unavailable_capability(),
                cancel: unavailable_capability(),
                continue_session: unavailable_capability(),
                restricted_policy: unavailable_capability(),
            }
            };
            Ok(WorkerExecutorAvailabilityListV1 {
                schema: hiroute_application_api::WORKER_EXECUTOR_AVAILABILITY_SCHEMA_V1.into(),
                executors: vec![
                    unavailable(hiroute_domain::delegation::WorkerHarnessV1::CodexCli),
                    unavailable(hiroute_domain::delegation::WorkerHarnessV1::ClaudeCode),
                    unavailable(hiroute_domain::delegation::WorkerHarnessV1::QoderCli),
                    unavailable(hiroute_domain::delegation::WorkerHarnessV1::Pi),
                    unavailable(hiroute_domain::delegation::WorkerHarnessV1::DeepseekHarness),
                ],
            })
        }

        fn worker_plans(
            &self,
            _: &WorkerPlansRequestV1,
        ) -> Result<WorkPlanListV1, DelegationErrorV1> {
            Ok(WorkPlanListV1 {
                schema: "hiroute.work-plan-list/v1".to_owned(),
                plans: Vec::new(),
            })
        }
    }

    fn unavailable_capability() -> hiroute_application_api::WorkerExecutorCapabilityAvailabilityV1 {
        hiroute_application_api::WorkerExecutorCapabilityAvailabilityV1 {
            state: hiroute_application_api::WorkerExecutorAvailabilityStateV1::Unavailable,
            reason: Some(
                hiroute_application_api::WorkerExecutorAvailabilityReasonV1::InstallationNotConfigured,
            ),
        }
    }
    #[test]
    fn worker_invocation_is_instance_scoped_and_rejects_agent_selectors() {
        let tasks = DelegationTasks::new(Arc::new(Tasks));
        let response = invoke_worker(
            &tasks,
            serde_json::json!({}),
            "worker-plans".to_owned(),
            WorkerPlansRequestV1::valid,
            |port, request| port.worker_plans(request),
        );
        assert_eq!(response.status, MachineStatus::Succeeded);
        let rejected = invoke_worker(
            &tasks,
            serde_json::json!({"agent_id":"codex"}),
            "worker-plans-forged".to_owned(),
            WorkerPlansRequestV1::valid,
            |port, request| port.worker_plans(request),
        );
        assert_eq!(rejected.status, ErrorCode::InvalidArguments.status());
    }

    #[test]
    fn executor_availability_has_no_plan_or_agent_selector_dependency() {
        let tasks = DelegationTasks::new(Arc::new(Tasks));
        let response = invoke_worker(
            &tasks,
            serde_json::json!({}),
            "worker-executors".to_owned(),
            |_| true,
            |port, _: &hiroute_application_api::ClientEmptyRequestV1| {
                port.worker_executor_availability()
            },
        );
        assert_eq!(response.status, MachineStatus::Succeeded);
        let value = response.data.unwrap();
        assert!(value.get("plans").is_none());
        assert!(value.get("agent_id").is_none());
        assert_eq!(
            value["executors"]
                .as_array()
                .unwrap()
                .iter()
                .map(|executor| executor["harness"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "codex_cli",
                "claude_code",
                "qoder_cli",
                "pi",
                "deepseek_harness"
            ]
        );

        let rejected = invoke_worker(
            &tasks,
            serde_json::json!({"agent_id":"codex"}),
            "worker-executors-forged".to_owned(),
            |_| true,
            |port, _: &hiroute_application_api::ClientEmptyRequestV1| {
                port.worker_executor_availability()
            },
        );
        assert_eq!(rejected.status, ErrorCode::InvalidArguments.status());
    }

    #[test]
    fn dependency_check_error_names_stage_and_marks_only_transient_failures_retryable() {
        use hiroute_domain::delegation::{
            NativeDependencyCheckV1 as Check, NativeDependencyFailureReasonV1 as Reason,
            NativeDependencyFailureV1,
        };
        for check in [Check::ClaudeVersion, Check::PiNodeVersion, Check::PiSdk] {
            for reason in [
                Reason::Timeout,
                Reason::Unavailable,
                Reason::CleanupFailed,
                Reason::ProcessFailed,
                Reason::Unsupported,
                Reason::InvalidOutput,
            ] {
                let failure = NativeDependencyFailureV1 { check, reason };
                let response = failed_delegation(
                    DelegationErrorV1::DependencyCheckFailed(failure),
                    "probe".into(),
                );
                assert_eq!(response.status, MachineStatus::Unavailable);
                let error = response.error.unwrap();
                assert_eq!(error.code, ErrorCode::CapabilityUnavailable);
                assert_eq!(
                    error.message_key,
                    format!("worker.dependencies.{}.{}", check.key(), reason.key())
                );
                assert_eq!(
                    error.retryable,
                    matches!(
                        reason,
                        Reason::Timeout | Reason::Unavailable | Reason::CleanupFailed
                    )
                );
            }
        }
    }

    #[test]
    fn capacity_has_a_distinct_public_error_code() {
        assert_eq!(
            map_error(DelegationErrorV1::CapacityExceeded),
            ErrorCode::WorkerCapacityExceeded
        );
        assert_eq!(
            ErrorCode::WorkerCapacityExceeded.status(),
            MachineStatus::Unavailable
        );
    }

    struct ReadTasks(WorkerReadErrorV1);

    impl DelegationTaskPort for ReadTasks {
        fn worker_read(
            &self,
            _: &WorkerReadRequestV1,
        ) -> Result<WorkerReadDataV1, WorkerReadErrorV1> {
            Err(self.0.clone())
        }
    }

    #[test]
    fn worker_read_typed_gap_keeps_the_server_cursor_in_the_failure_action() {
        let tasks = DelegationTasks::new(Arc::new(ReadTasks(WorkerReadErrorV1::CursorGap {
            recovery_cursor: "signed-recovery".into(),
        })));
        let response = invoke_worker_read(
            &tasks,
            serde_json::json!({
                "run_id":"run/one",
                "cursor":"signed-old",
                "max_bytes":2048
            }),
            "read-gap".into(),
        );
        assert!(response.data.is_none());
        assert_eq!(response.status, MachineStatus::Conflict);
        assert_eq!(
            response.error.unwrap().message_key,
            "worker.read.cursor_gap"
        );
        assert_eq!(response.next_actions.len(), 2);
        assert_eq!(response.next_actions[0].command_id, "worker.read");
        assert_eq!(
            response.next_actions[0].input,
            serde_json::json!({
                "run_id":"run/one",
                "cursor":"signed-recovery",
                "max_bytes":2048
            })
        );
        assert_eq!(
            response.next_actions[0].reason_code,
            "worker.read.resume_gap"
        );
        assert_eq!(response.next_actions[1].command_id, "worker.status");
    }

    #[test]
    fn worker_read_errors_keep_stable_codes_without_public_details_payloads() {
        let cases = [
            (
                WorkerReadErrorV1::InvalidCursor,
                ErrorCode::InvalidArguments,
                "worker.read.invalid_cursor",
            ),
            (
                WorkerReadErrorV1::PageTooSmall,
                ErrorCode::InvalidArguments,
                "worker.read.page_too_small",
            ),
            (
                WorkerReadErrorV1::CursorStale,
                ErrorCode::RevisionConflict,
                "worker.read.cursor_stale",
            ),
            (
                WorkerReadErrorV1::Unavailable,
                ErrorCode::ObservationUnavailable,
                "worker.read.unavailable",
            ),
            (
                WorkerReadErrorV1::StorageUnavailable,
                ErrorCode::ObservationUnavailable,
                "worker.read.storage_unavailable",
            ),
        ];
        for (error, code, key) in cases {
            let tasks = DelegationTasks::new(Arc::new(ReadTasks(error)));
            let response =
                invoke_worker_read(&tasks, serde_json::json!({"run_id":"run/one"}), key.into());
            let error = response.error.unwrap();
            assert_eq!(error.code, code);
            assert_eq!(error.message_key, key);
            assert_eq!(error.details_schema, "hiroute.error-details/none-v1");
        }
    }
}
