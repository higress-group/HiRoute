//! Public daemon-instance Worker admission over the owner-only local transport.

use std::collections::BTreeSet;
use std::sync::Arc;

use hiroute_application::delegation::admission::DelegationAdmission;
use hiroute_application::publication::admission::{AdmissionAction, AdmissionSubject};
use hiroute_application::publication::versions::ExactPlanVersionPort;
use hiroute_application_api::{
    DELEGATION_ACCEPTED_SCHEMA_V1, DelegationAcceptedV1, WorkerExecRequestV1, derive_worker_title,
    normalize_worker_title,
};
use hiroute_domain::delegation::{
    DELEGATION_RUN_CONFIGURATION_VERSION_V1, DelegationAcceptanceV1, DelegationErrorV1,
    DelegationPlanBindingV1, DelegationRunConfigurationV1, DelegationRuntimePort,
    DelegationTaskTitleSourceV1, DelegationTaskTitleV1, DelegationTaskV1, RunProgressV1,
};
use hiroute_domain::{
    AgentPlanId, CanonicalDigest, PlanExecutionRef, PlanLifecycleV1, PlanVersionError,
    PublicationRepositoryPort, VersionOwnerRefV1, VersionReservationV1, WorkspaceId,
};
use hiroute_observation::managed_text::ManagedTextScope;

use super::{
    LocalControlAdapter,
    delegation_task_queries::authorize,
    delegation_tasks::{
        canonical_workspace_with_path, current_time_ms, deadline_seconds, run_owner,
    },
};

pub(super) struct CurrentPlan {
    pub(super) binding: DelegationPlanBindingV1,
    pub(super) reference: PlanExecutionRef,
}

/// Internal instance partition. It is derived by the daemon and never comes from a Worker DTO.
#[derive(Clone)]
pub(super) struct DelegationCallerContext {
    workspace_id: WorkspaceId,
}

impl DelegationCallerContext {
    pub(super) fn workspace_id(&self) -> &WorkspaceId {
        &self.workspace_id
    }
}

impl LocalControlAdapter {
    pub(super) fn worker_instance(&self) -> DelegationCallerContext {
        DelegationCallerContext {
            workspace_id: WorkspaceId::default(),
        }
    }

    pub(super) fn exec_worker(
        &self,
        request: &WorkerExecRequestV1,
    ) -> Result<DelegationAcceptedV1, DelegationErrorV1> {
        if !request.valid() {
            return Err(DelegationErrorV1::InvalidArguments);
        }
        let instance = self.worker_instance();
        let workspace = instance.workspace_id().clone();
        if let Some(existing) = DelegationRuntimePort::find_submission(
            self,
            &workspace,
            false,
            &request.submission_key,
        )? {
            let task = DelegationRuntimePort::task(self, &workspace, &existing.task_id)?
                .ok_or(DelegationErrorV1::StorageUnavailable)?;
            authorize(&instance, &task, &existing)?;
            existing.configuration.validate_for(&existing)?;
            let canonical_workspace_path = replay_workspace_path(
                &request.cwd,
                &existing.configuration.canonical_workspace_path,
            )?;
            let request_digest = keyed_worker_request_digest(
                &self.delegation_digest_authority,
                request,
                &canonical_workspace_path,
            )?;
            let title = self
                .task_content_projection(&task, current_time_ms()?)
                .title;
            return replay_or_conflict(existing, &request_digest, title);
        }

        let now_ms = current_time_ms()?;
        let (workspace_identity, canonical_workspace_path) =
            canonical_workspace_with_path(&request.cwd)?;
        let execution = request.execution(workspace_identity.root_identity.clone());
        let request_digest = keyed_worker_request_digest(
            &self.delegation_digest_authority,
            request,
            &canonical_workspace_path,
        )?;
        let (title_value, title_source) = match request.title.as_deref() {
            Some(title) => (
                normalize_worker_title(title).map_err(|_| DelegationErrorV1::InvalidArguments)?,
                DelegationTaskTitleSourceV1::Explicit,
            ),
            None => (
                derive_worker_title(&request.input.goal),
                DelegationTaskTitleSourceV1::Goal,
            ),
        };
        let title_lookup_key = self
            .delegation_digest_authority
            .worker_title_lookup_digest(&workspace, &title_value)
            .as_str()
            .to_owned();

        // This is only an optimistic snapshot. The same facts are read under the shared
        // admission gate immediately before retaining the exact version.
        let selected = self.gated_current_start_plan(
            &workspace,
            &request.plan_id,
            AdmissionAction::Start,
            &request.submission_key,
        )?;
        self.ensure_worker_dependencies(selected.binding.harness, false)?;
        let deadline_ms = now_ms
            .checked_add(execution.duration_ms)
            .ok_or(DelegationErrorV1::DeadlineExceeded)?;

        let task_id = random_id("task")?;
        let run_id = random_id("run")?;
        let lease_id = random_id("lease")?;
        let launch_nonce = random_id("launch")?;
        let body_scope = ManagedTextScope {
            workspace_id: workspace.clone(),
            task_id: task_id.clone(),
            run_id: run_id.clone(),
        };
        let body = self.persist_task_input(
            &workspace,
            &task_id,
            &run_id,
            &request.input.prompt(),
            now_ms,
        )?;
        let accepted_title = title_value.clone();
        let title = DelegationTaskTitleV1 {
            value: title_value,
            source: title_source,
            initial_body_ref: body.clone(),
        };
        let execution_owner_ref = format!("delegation-run/{run_id}");
        let task = DelegationTaskV1 {
            workspace_id: workspace.clone(),
            task_id: task_id.clone(),
            parent_task_ref: request.parent_task_ref.clone(),
            plan: selected.binding.clone(),
            workspace: workspace_identity,
            created_at_ms: now_ms,
            latest_run_id: run_id.clone(),
            latest_admission_sequence: 0,
            title: Some(title),
            session: None,
            resume_until_ms: 0,
            required_body_ids: vec![body.opaque_id.clone()],
            body_refs: vec![body],
            native_history_paths: vec![],
        };
        let run = hiroute_domain::delegation::DelegationRunV1 {
            workspace_id: workspace.clone(),
            task_id,
            run_id: run_id.clone(),
            ordinal: 1,
            continued_from: None,
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
            title_lookup_key: Some(title_lookup_key),
            expected_latest_run_id: None,
            admitted_at_ms: now_ms,
        };
        match canonical_workspace_with_path(&request.cwd) {
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
        let mut reservation_acquired = false;
        let admitted = DelegationAdmission {
            gate: Arc::clone(&self.plan_admission),
            safety: self.delegation_safety.as_ref(),
            runtime: self,
        }
        .accept(&acceptance, |guard, input| {
            let current = ExactPlanVersionPort::current_plan(self, guard, &request.plan_id)
                .map_err(map_plan_error)?;
            if current.status != PlanLifecycleV1::Enabled || current.reference != selected.reference
            {
                return Err(DelegationErrorV1::Conflict);
            }
            let refreshed = self.current_start_plan(&workspace, &request.plan_id)?;
            if refreshed.reference != selected.reference || refreshed.binding != selected.binding {
                return Err(DelegationErrorV1::Conflict);
            }
            let reservation = VersionReservationV1 {
                owner: reservation_owner.clone(),
                reference: current.reference,
                expires_at_unix: deadline_seconds(input.run.deadline_ms)?,
            };
            let version = ExactPlanVersionPort::acquire_exact(self, guard, &reservation)
                .map_err(map_plan_error)?;
            reservation_acquired = true;
            if !version.configuration.delegation_enabled
                || version.reference != selected.reference
                || version.configuration.work.as_ref().is_none_or(|work| {
                    CanonicalDigest::of(work)
                        .map(|digest| digest != selected.binding.harness_configuration_digest)
                        .unwrap_or(true)
                })
            {
                return Err(DelegationErrorV1::Conflict);
            }
            Ok(())
        });
        match admitted {
            Ok(accepted) if accepted.run_id == run_id => Ok(DelegationAcceptedV1 {
                schema: DELEGATION_ACCEPTED_SCHEMA_V1.to_owned(),
                task_id: accepted.task_id,
                run_id: accepted.run_id,
                title: Some(accepted_title),
                state: accepted.progress.state,
                state_revision: accepted.progress.revision,
                replayed: false,
            }),
            Ok(accepted) => {
                let cleanup = self.discard_task_input(&body_scope, now_ms);
                let release = release_reservation(self, &workspace, &reservation_owner);
                cleanup?;
                release?;
                let task = DelegationRuntimePort::task(self, &workspace, &accepted.task_id)?
                    .ok_or(DelegationErrorV1::StorageUnavailable)?;
                let title = self
                    .task_content_projection(&task, current_time_ms()?)
                    .title;
                replay_or_conflict(accepted, &request_digest, title)
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

    pub(super) fn current_start_plan(
        &self,
        workspace: &WorkspaceId,
        plan_id: &AgentPlanId,
    ) -> Result<CurrentPlan, DelegationErrorV1> {
        let stores = self
            .stores_lock()
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?;
        let control = stores.control();
        let record = control
            .active_publication(workspace)
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?
            .ok_or(DelegationErrorV1::CapabilityUnavailable)?;
        let publication = record
            .verify()
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?;
        let head = control
            .plan_head(workspace, plan_id)
            .map_err(map_plan_error)?
            .ok_or(DelegationErrorV1::CapabilityUnavailable)?;
        if head.status != PlanLifecycleV1::Enabled {
            return Err(DelegationErrorV1::CapabilityUnavailable);
        }
        let version = control
            .lookup_exact_plan_version(&head.reference)
            .map_err(map_plan_error)?;
        if !version.configuration.delegation_enabled {
            return Err(DelegationErrorV1::CapabilityUnavailable);
        }
        let Some(work) = version.configuration.work.clone() else {
            return Err(DelegationErrorV1::CapabilityUnavailable);
        };
        let published = publication
            .plans
            .iter()
            .find(|plan| plan.agent_plan_id() == plan_id)
            .ok_or(DelegationErrorV1::CapabilityUnavailable)?;
        if published.body.agent_plan_revision != head.reference.content_revision
            || published.model_alias() != &head.model_alias
        {
            return Err(DelegationErrorV1::Conflict);
        }
        let harness = work.harness;
        let configuration_digest =
            CanonicalDigest::of(&work).map_err(|_| DelegationErrorV1::InvalidArguments)?;
        Ok(CurrentPlan {
            binding: DelegationPlanBindingV1 {
                authority_id: publication.authority_id,
                plan_id: plan_id.clone(),
                plan_revision: head.reference.content_revision,
                plan_digest: head.reference.content_digest.clone(),
                publication_revision: record.publication_revision.get(),
                publication_digest: record.digest,
                exact_reference: exact_reference(&head.reference),
                model_alias: head.model_alias.as_str().to_owned(),
                harness,
                harness_configuration_digest: configuration_digest,
            },
            reference: head.reference,
        })
    }

    pub(super) fn gated_current_start_plan(
        &self,
        workspace: &WorkspaceId,
        plan_id: &AgentPlanId,
        action: AdmissionAction,
        stable_action_id: &str,
    ) -> Result<CurrentPlan, DelegationErrorV1> {
        let scope = BTreeSet::from([AdmissionSubject::Plan(plan_id.clone())]);
        let _guard = self
            .plan_admission
            .enter(workspace, &scope, action, stable_action_id)
            .map_err(|_| DelegationErrorV1::PermissionDenied)?;
        self.current_start_plan(workspace, plan_id)
    }

    pub(super) fn ensure_worker_dependencies(
        &self,
        harness: hiroute_domain::delegation::WorkerHarnessV1,
        continuing: bool,
    ) -> Result<(), DelegationErrorV1> {
        let selection =
            crate::delegation::installation::WorkerInstallationSelectionSource::selection(
                self, harness,
            )?
            .ok_or(DelegationErrorV1::DependenciesMissing)?;
        crate::delegation::installation::validate_persisted_installation_for_run(
            &selection.config,
            continuing,
        )
        .map(|_| ())
    }
}

fn release_reservation(
    adapter: &LocalControlAdapter,
    workspace: &WorkspaceId,
    owner: &VersionOwnerRefV1,
) -> Result<(), DelegationErrorV1> {
    ExactPlanVersionPort::release(adapter, workspace, owner).map_err(map_plan_error)
}

fn replay_or_conflict(
    run: hiroute_domain::delegation::DelegationRunV1,
    request_digest: &CanonicalDigest,
    title: Option<String>,
) -> Result<DelegationAcceptedV1, DelegationErrorV1> {
    if &run.request_digest != request_digest {
        return Err(DelegationErrorV1::Conflict);
    }
    Ok(DelegationAcceptedV1 {
        schema: DELEGATION_ACCEPTED_SCHEMA_V1.to_owned(),
        task_id: run.task_id,
        run_id: run.run_id,
        title,
        state: run.progress.state,
        state_revision: run.progress.revision,
        replayed: true,
    })
}

fn keyed_worker_request_digest(
    authority: &hiroute_observation::DigestAuthority,
    request: &WorkerExecRequestV1,
    canonical_workspace_path: &str,
) -> Result<CanonicalDigest, DelegationErrorV1> {
    let value = serde_json::json!({
        "operation": "exec",
        "plan_id": request.plan_id,
        "canonical_workspace_path": canonical_workspace_path,
        "permission_policy": request.permission_policy,
        "run_timeout_secs": request.run_timeout_secs,
        "input": request.input,
        "title_intent": match request.title.as_deref() {
            Some(title) => serde_json::json!({
                "source": "explicit",
                "value": normalize_worker_title(title)
                    .map_err(|_| DelegationErrorV1::InvalidArguments)?,
            }),
            None => serde_json::json!({"source": "goal"}),
        },
        "parent_task_ref": request.parent_task_ref,
    });
    let canonical = hiroute_domain::canonicalize_json(value);
    let bytes = serde_json::to_vec(&canonical).map_err(|_| DelegationErrorV1::InvalidArguments)?;
    Ok(authority.delegation_request_digest(&bytes))
}

pub(super) fn replay_workspace_path(
    requested: &str,
    accepted_canonical_path: &str,
) -> Result<String, DelegationErrorV1> {
    if requested == accepted_canonical_path {
        Ok(accepted_canonical_path.to_owned())
    } else {
        canonical_workspace_with_path(requested).map(|(_, path)| path)
    }
}

fn random_id(prefix: &str) -> Result<String, DelegationErrorV1> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| DelegationErrorV1::CapabilityUnavailable)?;
    let mut value = String::with_capacity(prefix.len() + 33);
    value.push_str(prefix);
    value.push('/');
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut value, "{byte:02x}").expect("writing to String cannot fail");
    }
    Ok(value)
}

fn exact_reference(reference: &PlanExecutionRef) -> String {
    format!(
        "plan/{}/revision/{}/digest/{}",
        reference.plan_id.as_str(),
        reference.content_revision,
        reference.content_digest
    )
}

fn map_plan_error(error: PlanVersionError) -> DelegationErrorV1 {
    match error {
        PlanVersionError::Invalid => DelegationErrorV1::InvalidArguments,
        PlanVersionError::Conflict | PlanVersionError::Stale => DelegationErrorV1::Conflict,
        PlanVersionError::Unavailable
        | PlanVersionError::Disabled
        | PlanVersionError::RecoveryRequired => DelegationErrorV1::CapabilityUnavailable,
        PlanVersionError::Retained | PlanVersionError::StorageUnavailable => {
            DelegationErrorV1::StorageUnavailable
        }
    }
}
