//! Exact-session continuation admission. No latest-plan or `session/new` fallback exists here.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use hiroute_application::delegation::admission::DelegationAdmission;
use hiroute_application::publication::admission::AdmissionAction;
use hiroute_application::publication::versions::ExactPlanVersionPort;
use hiroute_application_api::{
    DELEGATION_ACCEPTED_SCHEMA_V1, DelegationAcceptedV1, WorkerContinueRequestV1,
};
use hiroute_domain::delegation::{
    DELEGATION_RUN_CONFIGURATION_VERSION_V1, DelegationAcceptanceV1, DelegationErrorV1,
    DelegationRunConfigurationV1, DelegationRunV1, DelegationRuntimePort, RunProgressV1,
    RunStateV1,
};
use hiroute_domain::{
    CanonicalDigest, PlanExecutionRef, PlanLifecycleV1, PlanVersionError, VersionOwnerKindV1,
    VersionOwnerPurposeV1, VersionOwnerRefV1, VersionReservationV1, WorkspaceId,
};
use hiroute_observation::managed_text::{
    ManagedTextRef, ManagedTextScope, ManagedTextState, PAGE_BYTES,
};

use crate::delegation::content::read_required_body;

use super::{
    LocalControlAdapter,
    delegation_task_queries::authorize,
    delegation_tasks::{canonical_workspace, canonical_workspace_with_path},
    delegation_worker::replay_workspace_path,
};

impl LocalControlAdapter {
    pub(super) fn continue_worker(
        &self,
        request: &WorkerContinueRequestV1,
    ) -> Result<(DelegationAcceptedV1, PathBuf), DelegationErrorV1> {
        if !request.valid() {
            return Err(DelegationErrorV1::InvalidArguments);
        }
        // The executor takes the same owned lease before publishing a terminal result and keeps
        // it until exact native history is retained. Once succeeded is visible, waiting here can
        // no longer observe the half-finalized task. Durable records remain authoritative.
        let _finalization = self.delegation_finalization.acquire();
        let instance = self.worker_instance();
        let workspace = instance.workspace_id().clone();
        if let Some(existing) =
            DelegationRuntimePort::find_submission(self, &workspace, true, &request.submission_key)?
        {
            let task = DelegationRuntimePort::task(self, &workspace, &existing.task_id)?
                .ok_or(DelegationErrorV1::StorageUnavailable)?;
            authorize(&instance, &task, &existing)?;
            existing.configuration.validate_for(&existing)?;
            let path = request.cwd.as_deref().map_or_else(
                || Ok(existing.configuration.canonical_workspace_path.clone()),
                |requested| {
                    replay_workspace_path(
                        requested,
                        &existing.configuration.canonical_workspace_path,
                    )
                },
            )?;
            let request_digest =
                keyed_request_digest(&self.delegation_digest_authority, request, &path)?;
            let title = self
                .task_content_projection(&task, current_time_ms()?)
                .title;
            return replay_or_conflict(existing, &request_digest, title)
                .map(|accepted| (accepted, PathBuf::from(path)));
        }

        let requested_workspace = request
            .cwd
            .as_deref()
            .map(canonical_workspace_with_path)
            .transpose()?;
        let now_ms = current_time_ms()?;
        let prior_task = DelegationRuntimePort::task(self, &workspace, &request.task_id)?
            .ok_or(DelegationErrorV1::NotFound)?;
        if prior_task.latest_run_id != request.expected_latest_run_id
            || prior_task.resume_until_ms <= now_ms
            || prior_task.native_history_paths.is_empty()
            || prior_task
                .session
                .as_ref()
                .and_then(|session| session.native_session_id.as_ref())
                .is_none()
        {
            return Err(DelegationErrorV1::ResumeUnavailable);
        }
        let prior = DelegationRuntimePort::run(self, &workspace, &request.expected_latest_run_id)?
            .ok_or(DelegationErrorV1::ResumeUnavailable)?;
        if prior.task_id != prior_task.task_id
            || prior.progress.state != RunStateV1::Succeeded
            || !prior.progress.workspace_releasable()
        {
            return Err(DelegationErrorV1::ResumeUnavailable);
        }
        prior.configuration.validate_for(&prior)?;
        authorize(&instance, &prior_task, &prior)?;
        validate_resume_bodies(self, &prior_task, now_ms)?;
        let visible_title = self.task_content_projection(&prior_task, now_ms).title;
        let canonical_workspace_path = prior.configuration.canonical_workspace_path.clone();
        if requested_workspace
            .as_ref()
            .is_some_and(|(identity, path)| {
                identity != &prior_task.workspace || path != &canonical_workspace_path
            })
        {
            return Err(DelegationErrorV1::PermissionDenied);
        }
        if canonical_workspace(&canonical_workspace_path)? != prior_task.workspace {
            return Err(DelegationErrorV1::PermissionDenied);
        }
        self.gated_current_start_plan(
            &workspace,
            &prior_task.plan.plan_id,
            AdmissionAction::Continue,
            &request.submission_key,
        )
        .map_err(|_| DelegationErrorV1::ResumeUnavailable)?;
        self.ensure_worker_dependencies(prior_task.plan.harness, true)?;
        let execution = request.execution(prior_task.workspace.root_identity.clone());
        let deadline_ms = now_ms
            .checked_add(execution.duration_ms)
            .ok_or(DelegationErrorV1::DeadlineExceeded)?;
        let request_digest = keyed_request_digest(
            &self.delegation_digest_authority,
            request,
            &canonical_workspace_path,
        )?;
        let workspace_path = PathBuf::from(&canonical_workspace_path);

        let run_id = random_id("run")?;
        let lease_id = random_id("lease")?;
        let launch_nonce = random_id("launch")?;
        let ordinal = prior
            .ordinal
            .checked_add(1)
            .ok_or(DelegationErrorV1::Conflict)?;
        if prior_task.body_refs.len() >= 16 {
            return Err(DelegationErrorV1::ResumeUnavailable);
        }
        let body_scope = ManagedTextScope {
            workspace_id: workspace.clone(),
            task_id: prior_task.task_id.clone(),
            run_id: run_id.clone(),
        };
        let body = self.persist_task_input(
            &workspace,
            &prior_task.task_id,
            &run_id,
            &request.input.prompt(),
            now_ms,
        )?;
        let mut task = prior_task.clone();
        task.latest_run_id = run_id.clone();
        task.required_body_ids.push(body.opaque_id.clone());
        task.body_refs.push(body);
        let execution_owner_ref = format!("delegation-run/{run_id}");
        let run = DelegationRunV1 {
            workspace_id: workspace.clone(),
            task_id: task.task_id.clone(),
            run_id: run_id.clone(),
            ordinal,
            continued_from: Some(prior.run_id.clone()),
            idempotency_key: request.submission_key.clone(),
            request_digest: request_digest.clone(),
            admission_sequence: 0,
            accepted_at_ms: None,
            execution_owner_ref: execution_owner_ref.clone(),
            lease_id,
            daemon_epoch: self.delegation_epoch.clone(),
            permit_id: format!("run-config/{run_id}"),
            permit_generation: 1,
            configuration: DelegationRunConfigurationV1 {
                format_version: DELEGATION_RUN_CONFIGURATION_VERSION_V1,
                scope_id: format!("run-config/{run_id}"),
                generation: 1,
                canonical_workspace_path,
                permission_policy: request.permission_policy,
            },
            execution,
            deadline_ms,
            lease_revoked: false,
            launch_nonce,
            process: None,
            session: None,
            progress: RunProgressV1::default(),
            stop_evidence: None,
            result_body: None,
            result_incomplete: false,
        };
        let acceptance = DelegationAcceptanceV1 {
            task,
            run,
            title_lookup_key: None,
            expected_latest_run_id: Some(prior.run_id),
            admitted_at_ms: now_ms,
        };
        match canonical_workspace_with_path(&acceptance.run.configuration.canonical_workspace_path)
        {
            Ok((current_workspace, current_path))
                if current_workspace == acceptance.task.workspace
                    && current_path == acceptance.run.configuration.canonical_workspace_path => {}
            Ok(_) => {
                self.discard_task_input(&body_scope, now_ms)?;
                return Err(DelegationErrorV1::PermissionDenied);
            }
            Err(error) => {
                self.discard_task_input(&body_scope, now_ms)?;
                return Err(error);
            }
        }
        let reservation_owner = run_owner(&execution_owner_ref);
        let reference = task_reference(&prior_task);
        let mut reservation_acquired = false;
        let admitted = DelegationAdmission {
            gate: Arc::clone(&self.plan_admission),
            safety: self.delegation_safety.as_ref(),
            runtime: self,
        }
        .accept(&acceptance, |guard, input| {
            self.current_start_plan(&workspace, &prior_task.plan.plan_id)
                .map_err(|_| DelegationErrorV1::ResumeUnavailable)?;
            let current = ExactPlanVersionPort::current_plan(self, guard, &prior_task.plan.plan_id)
                .map_err(map_plan_error)?;
            if current.status != PlanLifecycleV1::Enabled {
                return Err(DelegationErrorV1::ResumeUnavailable);
            }
            let reservation = VersionReservationV1 {
                owner: reservation_owner.clone(),
                reference: reference.clone(),
                expires_at_unix: deadline_seconds(input.run.deadline_ms)?,
            };
            let version = ExactPlanVersionPort::acquire_exact(self, guard, &reservation)
                .map_err(|_| DelegationErrorV1::ResumeUnavailable)?;
            reservation_acquired = true;
            if !version.configuration.delegation_enabled
                || version.reference != reference
                || version.configuration.work.as_ref().is_none_or(|work| {
                    CanonicalDigest::of(work)
                        .map(|digest| digest != prior_task.plan.harness_configuration_digest)
                        .unwrap_or(true)
                })
                || version.compiled.model_alias().as_str() != prior_task.plan.model_alias
            {
                return Err(DelegationErrorV1::ResumeUnavailable);
            }
            Ok(())
        });
        match admitted {
            Ok(accepted) if accepted.run_id == run_id => {
                let _ = ExactPlanVersionPort::release(self, &workspace, &task_owner(&prior_task));
                Ok((
                    accepted_view(accepted, false, visible_title),
                    workspace_path,
                ))
            }
            Ok(accepted) => {
                let cleanup = self.discard_task_input(&body_scope, now_ms);
                let release = release_reservation(self, &workspace, &reservation_owner);
                cleanup?;
                release?;
                replay_or_conflict(accepted, &request_digest, visible_title)
                    .map(|view| (view, workspace_path))
            }
            Err(error) => {
                let cleanup = self.discard_task_input(&body_scope, now_ms);
                let release = if reservation_acquired {
                    release_reservation(self, &workspace, &reservation_owner)
                } else {
                    Ok(())
                };
                cleanup?;
                release?;
                Err(error)
            }
        }
    }
}

fn validate_resume_bodies(
    adapter: &LocalControlAdapter,
    task: &hiroute_domain::delegation::DelegationTaskV1,
    now_ms: u64,
) -> Result<(), DelegationErrorV1> {
    if task.required_body_ids.len() != task.body_refs.len() || task.body_refs.is_empty() {
        return Err(DelegationErrorV1::ResumeUnavailable);
    }
    let now = i64::try_from(now_ms).map_err(|_| DelegationErrorV1::ResumeUnavailable)?;
    for body in &task.body_refs {
        let scope = ManagedTextScope {
            workspace_id: task.workspace_id.clone(),
            task_id: task.task_id.clone(),
            run_id: body.scope_run_id.clone(),
        };
        let reference = ManagedTextRef {
            opaque_id: body.opaque_id.clone(),
            scope: scope.clone(),
            visibility_generation: body.visibility_generation,
            original_retention_deadline_ms: body.original_retention_deadline_ms,
            state: ManagedTextState::Complete,
        };
        read_required_body(
            &adapter.delegation_observation,
            &scope,
            &reference,
            now,
            16 * PAGE_BYTES,
            |_| Ok(()),
        )?;
    }
    Ok(())
}

fn keyed_request_digest(
    authority: &hiroute_observation::DigestAuthority,
    request: &WorkerContinueRequestV1,
    canonical_workspace_path: &str,
) -> Result<CanonicalDigest, DelegationErrorV1> {
    let value = serde_json::json!({
        "operation": "continue",
        "task_id": request.task_id,
        "expected_latest_run_id": request.expected_latest_run_id,
        "canonical_workspace_path": canonical_workspace_path,
        "permission_policy": request.permission_policy,
        "run_timeout_secs": request.run_timeout_secs,
        "input": request.input,
    });
    let bytes = serde_json::to_vec(&hiroute_domain::canonicalize_json(value))
        .map_err(|_| DelegationErrorV1::InvalidArguments)?;
    Ok(authority.delegation_request_digest(&bytes))
}

fn replay_or_conflict(
    run: DelegationRunV1,
    request_digest: &CanonicalDigest,
    title: Option<String>,
) -> Result<DelegationAcceptedV1, DelegationErrorV1> {
    if &run.request_digest != request_digest {
        return Err(DelegationErrorV1::Conflict);
    }
    Ok(accepted_view(run, true, title))
}

fn accepted_view(
    run: DelegationRunV1,
    replayed: bool,
    title: Option<String>,
) -> DelegationAcceptedV1 {
    DelegationAcceptedV1 {
        schema: DELEGATION_ACCEPTED_SCHEMA_V1.into(),
        task_id: run.task_id,
        run_id: run.run_id,
        title,
        state: run.progress.state,
        state_revision: run.progress.revision,
        replayed,
    }
}

fn task_reference(task: &hiroute_domain::delegation::DelegationTaskV1) -> PlanExecutionRef {
    PlanExecutionRef {
        workspace_id: task.workspace_id.clone(),
        plan_id: task.plan.plan_id.clone(),
        content_revision: task.plan.plan_revision,
        content_digest: task.plan.plan_digest.clone(),
    }
}

pub(crate) fn task_owner(task: &hiroute_domain::delegation::DelegationTaskV1) -> VersionOwnerRefV1 {
    VersionOwnerRefV1 {
        kind: VersionOwnerKindV1::Task,
        owner_id: format!("delegation-task/{}", task.task_id),
        purpose: VersionOwnerPurposeV1::Continuation,
    }
}

fn run_owner(owner_id: &str) -> VersionOwnerRefV1 {
    VersionOwnerRefV1 {
        kind: VersionOwnerKindV1::Run,
        owner_id: owner_id.into(),
        purpose: VersionOwnerPurposeV1::Execution,
    }
}

fn release_reservation(
    adapter: &LocalControlAdapter,
    workspace: &WorkspaceId,
    owner: &VersionOwnerRefV1,
) -> Result<(), DelegationErrorV1> {
    ExactPlanVersionPort::release(adapter, workspace, owner).map_err(map_plan_error)
}

fn current_time_ms() -> Result<u64, DelegationErrorV1> {
    let value = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| DelegationErrorV1::DeadlineExceeded)?
        .as_millis();
    u64::try_from(value).map_err(|_| DelegationErrorV1::DeadlineExceeded)
}

fn deadline_seconds(deadline_ms: u64) -> Result<i64, DelegationErrorV1> {
    let rounded = deadline_ms
        .checked_add(999)
        .ok_or(DelegationErrorV1::DeadlineExceeded)?
        / 1_000;
    i64::try_from(rounded).map_err(|_| DelegationErrorV1::DeadlineExceeded)
}

fn random_id(prefix: &str) -> Result<String, DelegationErrorV1> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| DelegationErrorV1::CapabilityUnavailable)?;
    let mut value = format!("{prefix}/");
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut value, "{byte:02x}").expect("writing to String cannot fail");
    }
    Ok(value)
}

fn map_plan_error(error: PlanVersionError) -> DelegationErrorV1 {
    match error {
        PlanVersionError::Invalid => DelegationErrorV1::InvalidArguments,
        PlanVersionError::Conflict | PlanVersionError::Stale => DelegationErrorV1::Conflict,
        PlanVersionError::Unavailable
        | PlanVersionError::Disabled
        | PlanVersionError::RecoveryRequired => DelegationErrorV1::ResumeUnavailable,
        PlanVersionError::Retained | PlanVersionError::StorageUnavailable => {
            DelegationErrorV1::StorageUnavailable
        }
    }
}
