//! Sealed admission for command-specific typed planners.
//!
//! Application either reproduces a typed plan or seals a one-shot ordinary-publication
//! constructor that runs inside the writer. The production adapter may carry it to this coordinator, but cannot deserialize or
//! manufacture one. Admission therefore retains the exact generic ordering and the single
//! operation-journal writer boundary.

use hiroute_application_api::{
    APPLY_COMPUTE_SAVE_OPERATION_V2, APPLY_SUBSCRIPTION_CHECK_OPERATION_V2, ApplyRequestV1,
    CommandKind, command_by_id,
};
use hiroute_domain::{
    BeginOperationOutcome, CanonicalDigest, IdempotencyScopeV1, OperationId, OperationV1,
    ProtectedApplyCapability, RevisionSetV1, SecretMutationKind, TransactionPlanV1, WorkspaceId,
    compare_revisions,
};

use super::{
    AcceptedApply, TransactionCoordinator, TransactionError, VerifiedPrincipal,
    apply_request_digest,
};
use crate::change::validate_feature_gate;
use crate::compute_management::ComputePreparedDiscoveryGuardV1;

/// One command-specific plan reproduced by an Application-owned typed planner.
///
/// Fields and constructors are crate-private, and this type intentionally implements neither
/// `Deserialize` nor `Clone`; an adapter can only move the exact value back to Application's
/// transaction coordinator.
pub struct PreparedTransactionV1 {
    request: ApplyRequestV1,
    reproduced_digest: CanonicalDigest,
    plan: PreparedPlan,
    operation_kind: String,
    revalidate: Option<Box<dyn FnOnce() -> Result<(), TransactionError> + Send>>,
    discovery_guards: Vec<ComputePreparedDiscoveryGuardV1>,
}

enum PreparedPlan {
    Reproduced(Box<TransactionPlanV1>),
    Publication(Box<dyn FnOnce() -> Result<TransactionPlanV1, TransactionError> + Send>),
}

impl PreparedTransactionV1 {
    pub(crate) fn for_setup(
        request: ApplyRequestV1,
        reproduced_digest: CanonicalDigest,
        plan: TransactionPlanV1,
    ) -> Result<Self, TransactionError> {
        if request.spec != *plan.spec() || !prepared_command_is_allowed(&request.spec.command_id) {
            return Err(TransactionError::InvalidArguments);
        }
        let operation_kind = registered_apply_operation_kind(&request.spec.command_id)?;
        Ok(Self {
            request,
            reproduced_digest,
            plan: PreparedPlan::Reproduced(Box::new(plan)),
            operation_kind,
            revalidate: None,
            discovery_guards: Vec::new(),
        })
    }
    /// The registered restore descriptor has its own confirmation and idempotency scope.
    /// Only an exact restore-only settings plan can use that authority.
    pub(crate) fn for_agent_settings_restore(
        request: ApplyRequestV1,
        reproduced_digest: CanonicalDigest,
        plan: TransactionPlanV1,
    ) -> Result<Self, TransactionError> {
        if !settings_restore_plan(plan.spec()) {
            return Err(TransactionError::InvalidArguments);
        }
        let mut prepared = Self::for_setup(request, reproduced_digest, plan)?;
        prepared.operation_kind = command_by_id("agents.restore.apply")
            .filter(|descriptor| descriptor.kind == CommandKind::Apply)
            .ok_or(TransactionError::InvalidArguments)?
            .operation_id;
        Ok(prepared)
    }

    /// The source-level management API has a distinct public operation identity while reusing
    /// the registered compute connection command and its sealed transaction plan.
    pub(crate) fn for_compute_management(
        request: ApplyRequestV1,
        reproduced_digest: CanonicalDigest,
        plan: TransactionPlanV1,
        discovery_guards: Vec<ComputePreparedDiscoveryGuardV1>,
    ) -> Result<Self, TransactionError> {
        if plan.spec().command_id != "compute.connection.apply" {
            return Err(TransactionError::InvalidArguments);
        }
        let mut prepared = Self::for_setup(request, reproduced_digest, plan)?;
        prepared.operation_kind = APPLY_COMPUTE_SAVE_OPERATION_V2.to_owned();
        prepared.discovery_guards = discovery_guards;
        Ok(prepared)
    }

    /// Operation A has its own authority and durable identity. Its plan contains only the safe
    /// candidate/evidence binding; the protected native source is resolved by the adapter after
    /// this Operation has been durably admitted.
    pub(crate) fn for_subscription_check(
        request: ApplyRequestV1,
        reproduced_digest: CanonicalDigest,
        plan: TransactionPlanV1,
    ) -> Result<Self, TransactionError> {
        if request.spec != *plan.spec()
            || request.spec.command_id != hiroute_domain::COMPUTE_SUBSCRIPTION_CHECK_COMMAND_ID_V2
        {
            return Err(TransactionError::InvalidArguments);
        }
        Ok(Self {
            request,
            reproduced_digest,
            plan: PreparedPlan::Reproduced(Box::new(plan)),
            operation_kind: APPLY_SUBSCRIPTION_CHECK_OPERATION_V2.to_owned(),
            revalidate: None,
            discovery_guards: Vec::new(),
        })
    }

    pub(crate) fn for_worker_dependency_selection(
        request: ApplyRequestV1,
        reproduced_digest: CanonicalDigest,
        plan: TransactionPlanV1,
    ) -> Result<Self, TransactionError> {
        if request.spec != *plan.spec()
            || request.spec.command_id != "worker.dependencies.select"
            || plan.worker_dependency_selection().is_none()
            || !request.expected_revisions.dependencies.is_empty()
        {
            return Err(TransactionError::InvalidArguments);
        }
        Ok(Self {
            request,
            reproduced_digest,
            plan: PreparedPlan::Reproduced(Box::new(plan)),
            operation_kind: hiroute_application_api::WORKER_DEPENDENCIES_SELECT_OPERATION_V1
                .to_owned(),
            revalidate: None,
            discovery_guards: Vec::new(),
        })
    }

    pub(crate) fn for_plan_content(
        request: ApplyRequestV1,
        build: impl FnOnce() -> Result<TransactionPlanV1, TransactionError> + Send + 'static,
    ) -> Result<Self, TransactionError> {
        if request.spec.command_id != "routing.apply"
            || request
                .spec
                .desired_state
                .get("schema")
                .and_then(serde_json::Value::as_str)
                != Some(hiroute_application_api::PLAN_CONTENT_CHANGE_SCHEMA_V2)
        {
            return Err(TransactionError::InvalidArguments);
        }
        Ok(Self {
            reproduced_digest: request.accept_digest.clone(),
            request,
            plan: PreparedPlan::Publication(Box::new(build)),
            operation_kind: "ApplyAgentPlanChange".into(),
            revalidate: None,
            discovery_guards: Vec::new(),
        })
    }

    /// Application-owned typed fact reproduction, executed only inside the writer boundary.
    /// The durable plan carries effect identities; this callback is admission-only.
    pub(crate) fn with_revalidation(
        mut self,
        check: impl FnOnce() -> Result<(), TransactionError> + Send + 'static,
    ) -> Self {
        self.revalidate = Some(Box::new(check));
        self
    }
}

fn prepared_command_is_allowed(command_id: &str) -> bool {
    matches!(
        command_id,
        "agents.config-permissions.apply"
            | "compute.connection.apply"
            | "compute.credential.add"
            | "routing.apply"
            | "agents.connect.apply"
            | "prices.override.apply"
            | "agents.settings.apply"
    )
}

fn registered_apply_operation_kind(command_id: &str) -> Result<String, TransactionError> {
    // V2 settings use the existing connect descriptor/confirmation authority; their internal
    // journal command remains explicit so legacy V1 payloads cannot be reinterpreted.
    let descriptor_id = if command_id == "agents.settings.apply" {
        "agents.connect.apply"
    } else {
        command_id
    };
    command_by_id(descriptor_id)
        .filter(|descriptor| descriptor.kind == CommandKind::Apply)
        .map(|descriptor| descriptor.operation_id)
        .filter(|operation_kind| !operation_kind.is_empty())
        .ok_or(TransactionError::InvalidArguments)
}

fn settings_restore_plan(spec: &hiroute_domain::ChangeSpecV1) -> bool {
    spec.command_id == "agents.settings.apply"
        && serde_json::from_value::<hiroute_domain::AgentSettingsSpecV2>(spec.desired_state.clone())
            .is_ok_and(|settings| {
                settings.schema_version == hiroute_domain::AGENT_SETTINGS_SCHEMA_V2
                    && spec.resource_id.as_deref() == Some(settings.context_id.as_str())
                    && settings.is_restore_only()
            })
}

impl<'a, C, S, R, E, I> TransactionCoordinator<'a, C, S, R, E, I>
where
    C: hiroute_domain::ControlRepositoryPort
        + hiroute_domain::ComputeSourceControlPort
        + hiroute_domain::CredentialPoolControlPort
        + crate::ConnectionOptionAuthorizationPort,
    S: hiroute_domain::SecretStorePort,
    R: hiroute_domain::RuntimeStatePort,
    E: hiroute_domain::ExternalEffectPort,
    I: crate::ProtectedInputPort,
{
    /// Admits one setup step produced by a sealed typed planner, then leaves execution to the
    /// same fixed six-step journal used by every other Application mutation.
    pub fn accept_prepared(
        &self,
        workspace: &WorkspaceId,
        principal: &VerifiedPrincipal,
        prepared: PreparedTransactionV1,
    ) -> Result<AcceptedApply, TransactionError> {
        let PreparedTransactionV1 {
            request,
            reproduced_digest,
            plan,
            operation_kind,
            revalidate,
            discovery_guards,
        } = prepared;
        let plan = match plan {
            PreparedPlan::Publication(build) => {
                if revalidate.is_some()
                    || !discovery_guards.is_empty()
                    || operation_kind != "ApplyAgentPlanChange"
                    || request.spec.command_id != "routing.apply"
                {
                    return Err(TransactionError::InvalidArguments);
                }
                return self.accept_with_reproduced_plan(
                    workspace,
                    principal,
                    &operation_kind,
                    request,
                    move |_current, _request| {
                        let plan = build()?;
                        self.validate_prepared_external(&plan)?;
                        Ok((reproduced_digest, plan))
                    },
                );
            }
            PreparedPlan::Reproduced(plan) => *plan,
        };
        let worker_dependency_selection = plan.worker_dependency_selection().cloned();
        let kind_matches = if worker_dependency_selection.is_some() {
            operation_kind == hiroute_application_api::WORKER_DEPENDENCIES_SELECT_OPERATION_V1
                && plan.spec().command_id == "worker.dependencies.select"
        } else if plan.spec().command_id == hiroute_domain::COMPUTE_SUBSCRIPTION_CHECK_COMMAND_ID_V2
        {
            operation_kind == APPLY_SUBSCRIPTION_CHECK_OPERATION_V2
        } else {
            (registered_apply_operation_kind(&request.spec.command_id)? == operation_kind)
                || (operation_kind == "ApplyAgentConnectionRestore"
                    && settings_restore_plan(plan.spec()))
                || (operation_kind == APPLY_COMPUTE_SAVE_OPERATION_V2
                    && plan.spec().command_id == "compute.connection.apply")
        };
        if !kind_matches {
            return Err(TransactionError::InvalidArguments);
        }
        if let Some(change) = worker_dependency_selection {
            return self.accept_worker_dependency_selection(
                workspace,
                principal,
                request,
                reproduced_digest,
                plan,
                change,
                revalidate,
            );
        }
        self.accept_with_reproduced_plan(
            workspace,
            principal,
            &operation_kind,
            request,
            move |_current, _request| {
                if matches!(
                    plan.spec().command_id.as_str(),
                    "routing.apply" | "agents.connect.apply" | "agents.settings.apply"
                ) {
                    revalidate.ok_or(TransactionError::ChangePreviewStale)?()?;
                } else if let Some(check) = revalidate {
                    check()?;
                }
                for guard in discovery_guards {
                    self.protected_inputs
                        .validate_discovery_evidence(&guard.input_slot, &guard.evidence_digest)
                        .map_err(|_| TransactionError::ChangePreviewStale)?;
                    let mutation = plan
                        .secrets()
                        .iter()
                        .find(|mutation| {
                            mutation.kind() == SecretMutationKind::Upsert
                                && mutation.input_slot() == Some(guard.input_slot.as_str())
                        })
                        .ok_or(TransactionError::ChangePreviewStale)?;
                    let secret = self
                        .protected_inputs
                        .read_secret(&guard.input_slot)
                        .map_err(|_| TransactionError::ChangePreviewStale)?;
                    let observed = self.secrets.fingerprint(&secret)?;
                    if mutation.fingerprint() != Some(&observed) {
                        return Err(TransactionError::ChangePreviewStale);
                    }
                }
                self.validate_prepared_external(&plan)?;
                Ok((reproduced_digest, plan))
            },
        )
    }

    fn validate_prepared_external(&self, plan: &TransactionPlanV1) -> Result<(), TransactionError> {
        for intent in plan.external() {
            self.external.validate_external_admission(intent)?;
            if self
                .external
                .current_external_fingerprint(intent.target())?
                != intent.before_fingerprint().cloned()
            {
                return Err(TransactionError::ChangePreviewStale);
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn accept_worker_dependency_selection(
        &self,
        workspace: &WorkspaceId,
        principal: &VerifiedPrincipal,
        request: ApplyRequestV1,
        reproduced_digest: CanonicalDigest,
        plan: TransactionPlanV1,
        change: hiroute_domain::WorkerDependencySelectionChangeV1,
        revalidate: Option<Box<dyn FnOnce() -> Result<(), TransactionError> + Send>>,
    ) -> Result<AcceptedApply, TransactionError> {
        let _writer = self.admission.lock_writer()?;
        if !self
            .admission
            .writes_open
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err(TransactionError::RecoveryRequired);
        }
        if hiroute_domain::CHANGE_SPEC_SCHEMA_V1
            .negotiate(request.schema_version)
            .is_none()
            || !principal.local_control
            || request.apply_capability.is_some()
            || !request.expected_revisions.dependencies.is_empty()
            || request.expected_revisions.target != change.before_revision
        {
            return Err(TransactionError::InvalidArguments);
        }
        let operation_kind = hiroute_application_api::WORKER_DEPENDENCIES_SELECT_OPERATION_V1;
        let scope = IdempotencyScopeV1::new(
            principal.scope.clone(),
            operation_kind,
            request.idempotency_key.clone(),
        )?;
        let request_digest = apply_request_digest(&request)?;
        if let Some(existing) = self.control.operation_for_idempotency(workspace, &scope)? {
            if existing.accepted_digest == request.accept_digest {
                return Ok(AcceptedApply {
                    operation: existing,
                    existing: true,
                });
            }
            return Err(TransactionError::IdempotencyKeyReused);
        }
        let current_revision = self
            .control
            .worker_dependency_selection_revision(workspace, change.after_selection.harness)?;
        if current_revision != change.before_revision {
            return Err(TransactionError::RevisionConflict(
                hiroute_domain::RevisionMismatch::Target,
            ));
        }
        if let Some(check) = revalidate {
            check()?;
        }
        if reproduced_digest != request.accept_digest
            || plan.spec() != &request.spec
            || plan.worker_dependency_selection() != Some(&change)
        {
            return Err(TransactionError::ChangePreviewStale);
        }
        let operation_id = OperationId::derive(workspace, &scope, &request_digest);
        let operation = OperationV1::new(
            operation_id,
            workspace.clone(),
            scope,
            request_digest,
            request.accept_digest,
            request.expected_revisions,
            plan,
        )?;
        match self
            .control
            .begin_local_operation(&operation)
            .map_err(|error| {
                if error.code == hiroute_domain::PortErrorCode::Conflict {
                    TransactionError::ChangePreviewStale
                } else {
                    TransactionError::Port(error)
                }
            })? {
            BeginOperationOutcome::Created => Ok(AcceptedApply {
                operation,
                existing: false,
            }),
            BeginOperationOutcome::ExistingSame(existing) => Ok(AcceptedApply {
                operation: *existing,
                existing: true,
            }),
            BeginOperationOutcome::ExistingDifferent => Err(TransactionError::IdempotencyKeyReused),
            BeginOperationOutcome::RevisionChanged(mismatch) => {
                Err(TransactionError::RevisionConflict(mismatch))
            }
        }
    }

    pub(super) fn accept_with_reproduced_plan<F>(
        &self,
        workspace: &WorkspaceId,
        principal: &VerifiedPrincipal,
        operation_kind: &str,
        request: ApplyRequestV1,
        reproduce: F,
    ) -> Result<AcceptedApply, TransactionError>
    where
        F: FnOnce(
            &RevisionSetV1,
            &ApplyRequestV1,
        ) -> Result<(CanonicalDigest, TransactionPlanV1), TransactionError>,
    {
        let _writer = self.admission.lock_writer()?;
        if !self
            .admission
            .writes_open
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err(TransactionError::RecoveryRequired);
        }
        if hiroute_domain::CHANGE_SPEC_SCHEMA_V1
            .negotiate(request.schema_version)
            .is_none()
            || operation_kind.is_empty()
        {
            return Err(TransactionError::InvalidArguments);
        }
        let scope = IdempotencyScopeV1::new(
            principal.scope.clone(),
            operation_kind,
            request.idempotency_key.clone(),
        )?;
        let request_digest = apply_request_digest(&request)?;

        if let Some(existing) = self.control.operation_for_idempotency(workspace, &scope)? {
            if existing.accepted_digest == request.accept_digest {
                return Ok(AcceptedApply {
                    operation: existing,
                    existing: true,
                });
            }
            return Err(TransactionError::IdempotencyKeyReused);
        }

        // Routing changes have already passed their strict draft/content/lifecycle planner.
        // A REST classifier endpoint is a typed, validated routing field, not an arbitrary
        // effect URL from the generic setup command.
        if request.spec.command_id != "routing.apply" {
            validate_feature_gate(&request.spec.desired_state)?;
        }
        let current = self.control.current_revisions(workspace)?;
        compare_revisions(&request.expected_revisions, &current)
            .map_err(TransactionError::RevisionConflict)?;

        let authorization = if principal.local_control {
            if request.apply_capability.is_some() {
                return Err(TransactionError::InvalidArguments);
            }
            None
        } else {
            let capability = request
                .apply_capability
                .clone()
                .ok_or(TransactionError::CapabilityDenied)
                .and_then(|value| {
                    ProtectedApplyCapability::new(value)
                        .map_err(|_| TransactionError::CapabilityDenied)
                })?;
            Some(
                self.control
                    .verify_apply_authorization(
                        &capability,
                        workspace,
                        &principal.scope,
                        &scope.operation_kind,
                        &request.accept_digest,
                        &request.expected_revisions,
                    )
                    .map_err(|error| {
                        if error.code == hiroute_domain::PortErrorCode::PermissionDenied {
                            TransactionError::CapabilityDenied
                        } else {
                            TransactionError::Port(error)
                        }
                    })?,
            )
        };

        let (reproduced_digest, plan) = reproduce(&current, &request)?;
        if reproduced_digest != request.accept_digest || plan.spec() != &request.spec {
            return Err(TransactionError::ChangePreviewStale);
        }
        let operation_id = OperationId::derive(workspace, &scope, &request_digest);
        let operation = OperationV1::new(
            operation_id,
            workspace.clone(),
            scope,
            request_digest,
            request.accept_digest,
            request.expected_revisions,
            plan,
        )?;
        // Another accepted Operation can hold the durable claim between accept and run.
        // This atomic admission failed without consuming this capability or creating a row.
        // Keep this conversion here: post-admission effect failures retain recovery semantics.
        let admission = (if principal.local_control {
            self.control.begin_local_operation(&operation)
        } else {
            self.control.begin_operation(
                &operation,
                authorization.as_ref().expect("protected authorization"),
            )
        })
        .map_err(|error| {
            if error.code == hiroute_domain::PortErrorCode::Conflict {
                TransactionError::ChangePreviewStale
            } else {
                TransactionError::Port(error)
            }
        })?;
        match admission {
            BeginOperationOutcome::Created => Ok(AcceptedApply {
                operation,
                existing: false,
            }),
            BeginOperationOutcome::ExistingSame(existing) => Ok(AcceptedApply {
                operation: *existing,
                existing: true,
            }),
            BeginOperationOutcome::ExistingDifferent => Err(TransactionError::IdempotencyKeyReused),
            BeginOperationOutcome::RevisionChanged(mismatch) => {
                Err(TransactionError::RevisionConflict(mismatch))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepared_agent_connection_uses_registered_operation_kind() {
        for command in ["agents.connect.apply", "agents.settings.apply"] {
            assert_eq!(
                registered_apply_operation_kind(command).unwrap(),
                "ApplyAgentConnectionChange"
            );
        }
    }

    #[test]
    fn prepared_kind_resolution_fails_closed() {
        assert!(registered_apply_operation_kind("agents.connect.preview").is_err());
        assert!(registered_apply_operation_kind("unregistered.apply").is_err());
    }
}
