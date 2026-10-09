//! Production adapter for the one durable delegation admission point.
//!
//! This module deliberately owns no worker process.  It bridges the already-tested Application
//! admission protocol to the one Control/Runtime storage set, persists the bounded task body in
//! the managed-text store, and retains the exact Plan version before runtime acceptance.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use hiroute_application::delegation::tasks::{DelegationTaskPort, WorkerReadErrorV1};
use hiroute_application::publication::versions::ExactPlanVersionPort;
use hiroute_application_api::{
    DelegationAcceptedV1, DelegationCallerV1, DelegationCancelRequestV1, DelegationGetRequestV1,
    DelegationResultRequestV1, DelegationStartRequestV1, DelegationWaitRequestV1,
    MAX_DELEGATION_INPUT_BYTES, WorkPlanListV1, WorkPlanViewV1, WorkerCancelRequestV1,
    WorkerContinueRequestV1, WorkerDependenciesDiscoverRequestV1,
    WorkerDependenciesSelectRequestV1, WorkerDependenciesViewV1, WorkerExecRequestV1,
    WorkerExecutorAvailabilityListV1, WorkerListRequestV1, WorkerPlansRequestV1, WorkerReadDataV1,
    WorkerReadRequestV1, WorkerResidualConfirmRequestV1, WorkerResultRequestV1, WorkerSettingsV1,
    WorkerStatusRequestV1, WorkerWaitRequestV1,
};
use hiroute_domain::delegation::{
    DelegationBodyRefV1, DelegationCheckpointV1, DelegationErrorV1, DelegationPermitMutationV1,
    DelegationPermitStorePort, DelegationRuntimePort, DelegationWorkspaceV1, RunCleanupV1,
    RunEventV1, RunStateV1, WorkerConcurrencySettingsV1,
};
use hiroute_domain::{
    PlanExecutionRef, VersionOwnerKindV1, VersionOwnerPurposeV1, VersionOwnerRefV1,
    VersionReservationV1, WorkspaceId,
};
use hiroute_observation::managed_text::{ManagedTextPurpose, ManagedTextScope};

use crate::delegation::content::RunBodyWriter;
use crate::delegation::executor::DelegationRunExecutor;

use super::{LocalControlAdapter, delegation_worker::DelegationCallerContext};

/// Product composition wraps durable admission with the current daemon epoch's one-way wake.
/// The durable adapter remains the sole acceptance authority; a wake failure never rewrites an
/// accepted response or triggers a second submission, and the executor records any launch fact
/// it can durably establish.
pub(super) struct ScheduledDelegationTaskPort {
    adapter: Arc<LocalControlAdapter>,
    executor: Arc<DelegationRunExecutor>,
}

impl ScheduledDelegationTaskPort {
    pub(super) fn new(
        adapter: Arc<LocalControlAdapter>,
        executor: Arc<DelegationRunExecutor>,
    ) -> Self {
        Self { adapter, executor }
    }
}

impl DelegationTaskPort for ScheduledDelegationTaskPort {
    fn worker_settings(&self) -> Result<WorkerSettingsV1, DelegationErrorV1> {
        DelegationTaskPort::worker_settings(self.adapter.as_ref())
    }

    fn set_worker_settings(
        &self,
        settings: &WorkerSettingsV1,
    ) -> Result<WorkerSettingsV1, DelegationErrorV1> {
        DelegationTaskPort::set_worker_settings(self.adapter.as_ref(), settings)
    }

    fn worker_executor_availability(
        &self,
    ) -> Result<WorkerExecutorAvailabilityListV1, DelegationErrorV1> {
        DelegationTaskPort::worker_executor_availability(self.adapter.as_ref())
    }
    fn worker_plans(
        &self,
        request: &WorkerPlansRequestV1,
    ) -> Result<WorkPlanListV1, DelegationErrorV1> {
        DelegationTaskPort::worker_plans(self.adapter.as_ref(), request)
    }
    fn worker_dependencies_discover(
        &self,
        request: &WorkerDependenciesDiscoverRequestV1,
    ) -> Result<WorkerDependenciesViewV1, DelegationErrorV1> {
        DelegationTaskPort::worker_dependencies_discover(self.adapter.as_ref(), request)
    }
    fn validate_worker_dependency_selection(
        &self,
        request: &WorkerDependenciesSelectRequestV1,
    ) -> Result<WorkerDependenciesSelectRequestV1, DelegationErrorV1> {
        DelegationTaskPort::validate_worker_dependency_selection(self.adapter.as_ref(), request)
    }
    fn worker_exec(
        &self,
        request: &WorkerExecRequestV1,
    ) -> Result<DelegationAcceptedV1, DelegationErrorV1> {
        let accepted = DelegationTaskPort::worker_exec(self.adapter.as_ref(), request)?;
        if !accepted.replayed {
            let workspace = WorkspaceId::default();
            let run =
                DelegationRuntimePort::run(self.adapter.as_ref(), &workspace, &accepted.run_id)?
                    .ok_or(DelegationErrorV1::StorageUnavailable)?;
            let _ = self.executor.wake(
                &workspace,
                &accepted.run_id,
                PathBuf::from(run.configuration.canonical_workspace_path),
            );
        }
        Ok(accepted)
    }
    fn worker_list(
        &self,
        request: &WorkerListRequestV1,
    ) -> Result<hiroute_application_api::DelegationListV1, DelegationErrorV1> {
        DelegationTaskPort::worker_list(self.adapter.as_ref(), request)
    }
    fn worker_status(
        &self,
        request: &WorkerStatusRequestV1,
    ) -> Result<hiroute_application_api::DelegationGetV1, DelegationErrorV1> {
        DelegationTaskPort::worker_status(self.adapter.as_ref(), request)
    }
    fn worker_wait(
        &self,
        request: &WorkerWaitRequestV1,
    ) -> Result<hiroute_application_api::DelegationWaitV1, DelegationErrorV1> {
        DelegationTaskPort::worker_wait(self.adapter.as_ref(), request)
    }
    fn worker_result(
        &self,
        request: &WorkerResultRequestV1,
    ) -> Result<hiroute_application_api::DelegationResultV1, DelegationErrorV1> {
        DelegationTaskPort::worker_result(self.adapter.as_ref(), request)
    }
    fn worker_read(
        &self,
        request: &WorkerReadRequestV1,
    ) -> Result<WorkerReadDataV1, WorkerReadErrorV1> {
        DelegationTaskPort::worker_read(self.adapter.as_ref(), request)
    }
    fn worker_cancel(
        &self,
        request: &WorkerCancelRequestV1,
    ) -> Result<hiroute_application_api::DelegationCancelV1, DelegationErrorV1> {
        let result = DelegationTaskPort::worker_cancel(self.adapter.as_ref(), request)?;
        let _ = self.executor.wake_cancellation(&WorkspaceId::default());
        Ok(result)
    }
    fn confirm_worker_residual(
        &self,
        request: &WorkerResidualConfirmRequestV1,
    ) -> Result<hiroute_application_api::DelegationRunViewV1, DelegationErrorV1> {
        DelegationTaskPort::confirm_worker_residual(self.adapter.as_ref(), request)
    }
    fn worker_continue(
        &self,
        request: &WorkerContinueRequestV1,
    ) -> Result<DelegationAcceptedV1, DelegationErrorV1> {
        let (accepted, workspace_path) = self.adapter.continue_worker(request)?;
        if !accepted.replayed {
            let _ = self
                .executor
                .wake(&WorkspaceId::default(), &accepted.run_id, workspace_path);
        }
        Ok(accepted)
    }

    fn list(
        &self,
        principal: &hiroute_domain::VerifiedCollaborationPrincipal,
        request: &hiroute_application_api::DelegationListRequestV1,
    ) -> Result<hiroute_application_api::DelegationListV1, DelegationErrorV1> {
        DelegationTaskPort::list(self.adapter.as_ref(), principal, request)
    }
    fn get(
        &self,
        principal: &hiroute_domain::VerifiedCollaborationPrincipal,
        request: &hiroute_application_api::DelegationGetRequestV1,
    ) -> Result<hiroute_application_api::DelegationGetV1, DelegationErrorV1> {
        DelegationTaskPort::get(self.adapter.as_ref(), principal, request)
    }
    fn wait(
        &self,
        principal: &hiroute_domain::VerifiedCollaborationPrincipal,
        request: &hiroute_application_api::DelegationWaitRequestV1,
    ) -> Result<hiroute_application_api::DelegationWaitV1, DelegationErrorV1> {
        DelegationTaskPort::wait(self.adapter.as_ref(), principal, request)
    }
    fn result(
        &self,
        _principal: &hiroute_domain::VerifiedCollaborationPrincipal,
        _request: &hiroute_application_api::DelegationResultRequestV1,
    ) -> Result<hiroute_application_api::DelegationResultV1, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }
    fn cancel(
        &self,
        principal: &hiroute_domain::VerifiedCollaborationPrincipal,
        request: &hiroute_application_api::DelegationCancelRequestV1,
    ) -> Result<hiroute_application_api::DelegationCancelV1, DelegationErrorV1> {
        let result = DelegationTaskPort::cancel(self.adapter.as_ref(), principal, request)?;
        let _ = self.executor.wake_cancellation(principal.workspace_id());
        Ok(result)
    }
    fn continue_task(
        &self,
        _principal: &hiroute_domain::VerifiedCollaborationPrincipal,
        _request: &hiroute_application_api::DelegationContinueRequestV1,
    ) -> Result<DelegationAcceptedV1, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }
    fn start(
        &self,
        principal: &hiroute_domain::VerifiedCollaborationPrincipal,
        request: &DelegationStartRequestV1,
    ) -> Result<DelegationAcceptedV1, DelegationErrorV1> {
        let accepted = DelegationTaskPort::start(self.adapter.as_ref(), principal, request)?;
        if !accepted.replayed {
            // `LocalControlAdapter::start` already canonicalized this input before committing
            // acceptance; the executor independently verifies its opaque workspace identity.
            let _ = self.executor.wake(
                &request.caller.workspace_id,
                &accepted.run_id,
                PathBuf::from(&request.workspace_path),
            );
        }
        Ok(accepted)
    }
}

impl DelegationPermitStorePort for LocalControlAdapter {
    fn permit(
        &self,
        workspace: &WorkspaceId,
        id: &str,
    ) -> Result<Option<hiroute_domain::delegation::WorkspaceExecutionPermitV1>, DelegationErrorV1>
    {
        self.stores_lock()
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?
            .control()
            .permit(workspace, id)
    }

    fn prepare_permit(
        &self,
        mutation: &DelegationPermitMutationV1,
    ) -> Result<(), DelegationErrorV1> {
        self.stores_lock()
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?
            .control()
            .prepare_permit(mutation)
    }

    fn pending_permit_mutations(
        &self,
        workspace: &WorkspaceId,
    ) -> Result<Vec<DelegationPermitMutationV1>, DelegationErrorV1> {
        self.stores_lock()
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?
            .control()
            .pending_permit_mutations(workspace)
    }

    fn commit_permit(
        &self,
        mutation: &DelegationPermitMutationV1,
    ) -> Result<(), DelegationErrorV1> {
        self.stores_lock()
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?
            .control()
            .commit_permit(mutation)
    }

    fn permit_mutations(
        &self,
        workspace: &WorkspaceId,
    ) -> Result<Vec<DelegationPermitMutationV1>, DelegationErrorV1> {
        self.stores_lock()
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?
            .control()
            .permit_mutations(workspace)
    }
}

impl LocalControlAdapter {
    fn current_worker_executor_availability(
        &self,
    ) -> Result<WorkerExecutorAvailabilityListV1, DelegationErrorV1> {
        let availability = self
            .managed_agent_runtime
            .lock()
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?
            .as_ref()
            .map(|runtime| Arc::clone(&runtime.worker_executor_availability));
        Ok(availability.map_or_else(
            crate::delegation::installation::WorkerExecutorAvailabilityRegistry::runtime_unavailable,
            |availability| availability.snapshot(),
        ))
    }

    /// Reconstruct the complete durable owner set before an exact Plan can be retained again.
    /// A pre-existing run is never respawned here: it remains an occupied, recoverable record
    /// until a later explicit lifecycle reconciliation observes it.
    pub(super) fn reconcile_delegation_plan_versions(&self) -> Result<(), String> {
        let workspace = WorkspaceId::default();
        let runs = DelegationRuntimePort::unreconciled(self, &workspace)
            .map_err(|error| error.to_string())?;
        let now_millis = current_time_ms().map_err(|error| error.to_string())?;
        let resumable = DelegationRuntimePort::resumable_tasks(self, &workspace, now_millis)
            .map_err(|error| error.to_string())?;
        let mut owners = Vec::with_capacity(runs.len() + resumable.len());
        for run in runs {
            // The old producer persisted Accepted -> Cancelling at revision 2. Preparing
            // consumes revision 2 before spawn, so this exact witness cannot hide a process.
            // Later revisions and every process/prompt witness still require owned cleanup.
            if run.progress.state == RunStateV1::Cancelling
                && run.progress.revision == 2
                && run.progress.cancel_requested
                && run.lease_revoked
                && run.progress.cleanup == RunCleanupV1::Pending
                && !run.progress.process_running
                && !run.progress.prompt_may_have_executed
                && run.process.is_none()
                && run.session.is_none()
                && run.stop_evidence.is_none()
            {
                DelegationRuntimePort::checkpoint(
                    self,
                    &workspace,
                    &run.run_id,
                    2,
                    "startup-cancel-before-preparation",
                    &DelegationCheckpointV1::Progress {
                        event: RunEventV1::LaunchFailedBeforeSpawn,
                    },
                )
                .map_err(|error| error.to_string())?;
                continue;
            }
            let task = DelegationRuntimePort::task(self, &workspace, &run.task_id)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "active delegation run has no task record".to_owned())?;
            let reference = PlanExecutionRef {
                workspace_id: workspace.clone(),
                plan_id: task.plan.plan_id.clone(),
                content_revision: task.plan.plan_revision,
                content_digest: task.plan.plan_digest.clone(),
            };
            reference.validate().map_err(|error| error.to_string())?;
            owners.push(VersionReservationV1 {
                owner: run_owner(&run.execution_owner_ref),
                reference,
                expires_at_unix: deadline_seconds(run.deadline_ms)
                    .map_err(|error| error.to_string())?,
            });
        }
        for task in resumable {
            let reference = PlanExecutionRef {
                workspace_id: workspace.clone(),
                plan_id: task.plan.plan_id.clone(),
                content_revision: task.plan.plan_revision,
                content_digest: task.plan.plan_digest.clone(),
            };
            reference.validate().map_err(|error| error.to_string())?;
            owners.push(VersionReservationV1 {
                owner: super::delegation_continue::task_owner(&task),
                reference,
                expires_at_unix: deadline_seconds(task.resume_until_ms)
                    .map_err(|error| error.to_string())?,
            });
        }
        let now_seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "system clock is before Unix epoch".to_owned())?
            .as_secs();
        let now_seconds = i64::try_from(now_seconds)
            .map_err(|_| "system clock is outside Plan retention range".to_owned())?;
        ExactPlanVersionPort::reconcile(self, &workspace, &owners, now_seconds)
            .map_err(|error| error.to_string())
    }

    pub(super) fn persist_task_input(
        &self,
        workspace: &WorkspaceId,
        task_id: &str,
        run_id: &str,
        prompt: &str,
        now_ms: u64,
    ) -> Result<DelegationBodyRefV1, DelegationErrorV1> {
        if prompt.is_empty() || prompt.len() > MAX_DELEGATION_INPUT_BYTES {
            return Err(DelegationErrorV1::InvalidArguments);
        }
        let now = i64::try_from(now_ms).map_err(|_| DelegationErrorV1::DeadlineExceeded)?;
        let scope = ManagedTextScope {
            workspace_id: workspace.clone(),
            task_id: task_id.to_owned(),
            run_id: run_id.to_owned(),
        };
        let event_id = format!("delegation-input/{task_id}/{run_id}");
        let persisted: Result<DelegationBodyRefV1, DelegationErrorV1> = (|| {
            let mut writer = RunBodyWriter::create(
                Arc::clone(&self.delegation_observation),
                scope.clone(),
                ManagedTextPurpose::Goal,
                event_id,
                now,
                now,
            )?;
            writer.append(prompt.as_bytes(), now)?;
            let reference = writer.finish(now)?;
            Ok(DelegationBodyRefV1 {
                opaque_id: reference.opaque_id,
                scope_run_id: run_id.to_owned(),
                visibility_generation: reference.visibility_generation,
                original_retention_deadline_ms: reference.original_retention_deadline_ms,
            })
        })();
        match persisted {
            Ok(body) => Ok(body),
            Err(error) => {
                self.discard_task_input(&scope, now_ms)?;
                Err(error)
            }
        }
    }

    pub(super) fn discard_task_input(
        &self,
        scope: &ManagedTextScope,
        now_ms: u64,
    ) -> Result<(), DelegationErrorV1> {
        let now = i64::try_from(now_ms).map_err(|_| DelegationErrorV1::StorageUnavailable)?;
        let preview = self
            .delegation_observation
            .managed_text_delete_preview(scope, now, now)
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?;
        if preview.reference_count == 0 {
            return Ok(());
        }
        self.delegation_observation
            .managed_text_delete_apply(&preview)
            .map(|_| ())
            .map_err(|_| DelegationErrorV1::StorageUnavailable)
    }
}

impl DelegationTaskPort for LocalControlAdapter {
    fn worker_settings(&self) -> Result<WorkerSettingsV1, DelegationErrorV1> {
        let settings = DelegationRuntimePort::worker_concurrency_settings(self)?;
        Ok(WorkerSettingsV1::new(settings.max_concurrent))
    }

    fn set_worker_settings(
        &self,
        settings: &WorkerSettingsV1,
    ) -> Result<WorkerSettingsV1, DelegationErrorV1> {
        if !settings.valid() {
            return Err(DelegationErrorV1::InvalidArguments);
        }
        let effective = DelegationRuntimePort::set_worker_concurrency_settings(
            self,
            WorkerConcurrencySettingsV1 {
                max_concurrent: settings.max_concurrent,
            },
        )?;
        Ok(WorkerSettingsV1::new(effective.max_concurrent))
    }

    fn worker_executor_availability(
        &self,
    ) -> Result<WorkerExecutorAvailabilityListV1, DelegationErrorV1> {
        self.current_worker_executor_availability()
    }

    fn worker_plans(
        &self,
        _request: &WorkerPlansRequestV1,
    ) -> Result<WorkPlanListV1, DelegationErrorV1> {
        let metadata = self
            .current_worker_plan_metadata(&WorkspaceId::default())
            .map_err(|_| DelegationErrorV1::CapabilityUnavailable)?;
        let mut plans = metadata
            .into_iter()
            .filter_map(|plan| {
                let (harness, protocol) = plan.work?;
                Some(WorkPlanViewV1 {
                    agent_plan_id: plan.agent_plan_id,
                    alias: plan.alias,
                    display_name: plan.display_name,
                    purpose: plan.purpose,
                    harness,
                    protocol,
                    availability: plan.availability,
                    reason: plan.reason,
                })
            })
            .collect::<Vec<_>>();
        plans.sort_by(|left, right| left.agent_plan_id.cmp(&right.agent_plan_id));
        Ok(WorkPlanListV1 {
            schema: "hiroute.work-plan-list/v1".into(),
            plans,
        })
    }

    fn worker_dependencies_discover(
        &self,
        request: &WorkerDependenciesDiscoverRequestV1,
    ) -> Result<WorkerDependenciesViewV1, DelegationErrorV1> {
        crate::delegation::installation::discover_worker_dependencies(self, request)
    }

    fn validate_worker_dependency_selection(
        &self,
        request: &WorkerDependenciesSelectRequestV1,
    ) -> Result<WorkerDependenciesSelectRequestV1, DelegationErrorV1> {
        crate::delegation::installation::validate_selection(request)
    }

    fn worker_status(
        &self,
        request: &WorkerStatusRequestV1,
    ) -> Result<hiroute_application_api::DelegationGetV1, DelegationErrorV1> {
        let instance = self.worker_instance();
        self.get_delegation(
            &instance,
            &DelegationGetRequestV1 {
                caller: internal_worker_caller(&instance),
                task_id: request.task_id.clone(),
                run_id: request.run_id.clone(),
                submission_key: request.submission_key.clone(),
                submission_operation: request.submission_operation,
            },
        )
    }

    fn worker_list(
        &self,
        request: &WorkerListRequestV1,
    ) -> Result<hiroute_application_api::DelegationListV1, DelegationErrorV1> {
        let instance = self.worker_instance();
        self.list_delegations(
            &instance,
            request.cursor.as_deref(),
            request.effective_limit(),
            request.title.as_deref(),
        )
    }

    fn worker_wait(
        &self,
        request: &WorkerWaitRequestV1,
    ) -> Result<hiroute_application_api::DelegationWaitV1, DelegationErrorV1> {
        let instance = self.worker_instance();
        self.wait_delegation(
            &instance,
            &DelegationWaitRequestV1 {
                caller: internal_worker_caller(&instance),
                run_id: request.run_id.clone(),
                after_revision: request.after_revision,
                wait_ms: Some(request.wait_timeout_secs.saturating_mul(1_000)),
            },
        )
    }

    fn worker_result(
        &self,
        request: &WorkerResultRequestV1,
    ) -> Result<hiroute_application_api::DelegationResultV1, DelegationErrorV1> {
        let instance = self.worker_instance();
        self.read_delegation_result(
            &instance,
            &DelegationResultRequestV1 {
                caller: internal_worker_caller(&instance),
                run_id: request.run_id.clone(),
                offset: request.offset,
                max_bytes: request.max_bytes,
            },
        )
    }

    fn worker_read(
        &self,
        request: &WorkerReadRequestV1,
    ) -> Result<WorkerReadDataV1, WorkerReadErrorV1> {
        self.read_worker_progress(request)
    }

    fn worker_cancel(
        &self,
        request: &WorkerCancelRequestV1,
    ) -> Result<hiroute_application_api::DelegationCancelV1, DelegationErrorV1> {
        let instance = self.worker_instance();
        self.cancel_delegation(
            &instance,
            &DelegationCancelRequestV1 {
                caller: internal_worker_caller(&instance),
                run_id: request.run_id.clone(),
                idempotency_key: request.idempotency_key.clone(),
                reason: request.reason.clone(),
            },
        )
    }

    fn confirm_worker_residual(
        &self,
        request: &WorkerResidualConfirmRequestV1,
    ) -> Result<hiroute_application_api::DelegationRunViewV1, DelegationErrorV1> {
        self.confirm_delegation_residual(request)
    }
    fn worker_continue(
        &self,
        request: &WorkerContinueRequestV1,
    ) -> Result<DelegationAcceptedV1, DelegationErrorV1> {
        self.continue_worker(request).map(|(accepted, _)| accepted)
    }

    fn list(
        &self,
        _principal: &hiroute_domain::VerifiedCollaborationPrincipal,
        _request: &hiroute_application_api::DelegationListRequestV1,
    ) -> Result<hiroute_application_api::DelegationListV1, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }

    fn get(
        &self,
        _principal: &hiroute_domain::VerifiedCollaborationPrincipal,
        _request: &hiroute_application_api::DelegationGetRequestV1,
    ) -> Result<hiroute_application_api::DelegationGetV1, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }

    fn wait(
        &self,
        _principal: &hiroute_domain::VerifiedCollaborationPrincipal,
        _request: &hiroute_application_api::DelegationWaitRequestV1,
    ) -> Result<hiroute_application_api::DelegationWaitV1, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }

    fn cancel(
        &self,
        _principal: &hiroute_domain::VerifiedCollaborationPrincipal,
        _request: &hiroute_application_api::DelegationCancelRequestV1,
    ) -> Result<hiroute_application_api::DelegationCancelV1, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }

    fn continue_task(
        &self,
        _principal: &hiroute_domain::VerifiedCollaborationPrincipal,
        _request: &hiroute_application_api::DelegationContinueRequestV1,
    ) -> Result<DelegationAcceptedV1, DelegationErrorV1> {
        Err(DelegationErrorV1::CapabilityUnavailable)
    }

    fn worker_exec(
        &self,
        request: &WorkerExecRequestV1,
    ) -> Result<DelegationAcceptedV1, DelegationErrorV1> {
        self.exec_worker(request)
    }
}

fn internal_worker_caller(context: &DelegationCallerContext) -> DelegationCallerV1 {
    DelegationCallerV1 {
        workspace_id: context.workspace_id().clone(),
        context_id: "worker-instance".to_owned(),
        grant_id: "collaboration-grant/worker-instance".to_owned(),
        grant_generation: 1,
    }
}

pub(super) fn current_time_ms() -> Result<u64, DelegationErrorV1> {
    let milliseconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| DelegationErrorV1::DeadlineExceeded)?
        .as_millis();
    u64::try_from(milliseconds).map_err(|_| DelegationErrorV1::DeadlineExceeded)
}

pub(super) fn deadline_seconds(deadline_ms: u64) -> Result<i64, DelegationErrorV1> {
    let rounded = deadline_ms
        .checked_add(999)
        .ok_or(DelegationErrorV1::DeadlineExceeded)?
        / 1_000;
    i64::try_from(rounded).map_err(|_| DelegationErrorV1::DeadlineExceeded)
}

pub(super) fn run_owner(owner_id: &str) -> VersionOwnerRefV1 {
    VersionOwnerRefV1 {
        kind: VersionOwnerKindV1::Run,
        owner_id: owner_id.to_owned(),
        purpose: VersionOwnerPurposeV1::Execution,
    }
}

#[cfg(unix)]
pub(super) fn canonical_workspace(path: &str) -> Result<DelegationWorkspaceV1, DelegationErrorV1> {
    canonical_workspace_with_path(path).map(|(workspace, _)| workspace)
}

#[cfg(unix)]
pub(super) fn canonical_workspace_with_path(
    path: &str,
) -> Result<(DelegationWorkspaceV1, String), DelegationErrorV1> {
    use std::os::unix::fs::MetadataExt;

    let canonical = fs::canonicalize(path).map_err(|_| DelegationErrorV1::InvalidArguments)?;
    let canonical_path = canonical
        .to_str()
        .ok_or(DelegationErrorV1::InvalidArguments)?
        .to_owned();
    let metadata = fs::metadata(&canonical).map_err(|_| DelegationErrorV1::InvalidArguments)?;
    if !metadata.is_dir() {
        return Err(DelegationErrorV1::InvalidArguments);
    }
    let mut ancestry = Vec::new();
    for ancestor in canonical.ancestors() {
        let metadata = fs::metadata(ancestor).map_err(|_| DelegationErrorV1::InvalidArguments)?;
        ancestry.push(format!("inode/{}/{}", metadata.dev(), metadata.ino()));
    }
    ancestry.reverse();
    let root_identity = ancestry
        .last()
        .cloned()
        .ok_or(DelegationErrorV1::InvalidArguments)?;
    let workspace = DelegationWorkspaceV1 {
        root_identity,
        volume_identity: format!("volume/{}", metadata.dev()),
        ancestry,
    };
    workspace.validate()?;
    Ok((workspace, canonical_path))
}

#[cfg(not(unix))]
pub(super) fn canonical_workspace(_path: &str) -> Result<DelegationWorkspaceV1, DelegationErrorV1> {
    // Windows needs an owner-validated reparse-point/volume resolver; do not substitute a raw
    // string path while that capability is absent.
    Err(DelegationErrorV1::CapabilityUnavailable)
}

#[cfg(not(unix))]
pub(super) fn canonical_workspace_with_path(
    _path: &str,
) -> Result<(DelegationWorkspaceV1, String), DelegationErrorV1> {
    Err(DelegationErrorV1::CapabilityUnavailable)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn canonical_workspace_uses_opaque_file_identities_not_the_display_path() {
        let root = tempfile::tempdir().unwrap();
        let nested = root.path().join("workspace");
        fs::create_dir(&nested).unwrap();

        let derived = canonical_workspace(nested.to_str().unwrap()).unwrap();
        assert_eq!(derived.ancestry.last(), Some(&derived.root_identity));
        assert!(derived.root_identity.starts_with("inode/"));
        assert!(derived.volume_identity.starts_with("volume/"));
        assert!(!derived.root_identity.contains("workspace"));
        assert!(
            !derived
                .ancestry
                .iter()
                .any(|value| value.contains("workspace"))
        );
    }
}
