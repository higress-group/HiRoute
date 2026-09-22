//! Production adapters for Application's single transaction coordinator.

use hiroute_application::control::ApplicationMutationPort;
use hiroute_application::{
    ProtectedInputPort, TransactionCoordinator, TransactionError, VerifiedPrincipal,
};
use hiroute_application_api::{ApplyRequestV1, PreviewRequestV1, PreviewResultV1, PrincipalKind};
use hiroute_diagnostics::publication::{PublicationStage, measure};
use hiroute_domain::{
    AgentAccessGrantMaterial, AgentAccessGrantMutationV1, AgentAccessGrantRefV1,
    BeginOperationOutcome, CanonicalDigest, CompensationOutcome, ComputeSourceControlPort,
    ComputeSourceMutationV1, ControlRepositoryPort, CredentialPoolControlPort,
    CredentialPoolMutationV1, CredentialRefV1, EffectReconciliation, IdempotencyScopeV1,
    OperationId, OperationV1, OwnedEffectV1, PortError, PortErrorCode, PortResult,
    ProtectedApplyCapability, ProtectedSecret, RevisionSetV1, RuntimeMutationV1, RuntimeStatePort,
    SecretMutationV1, SecretStorePort, VerifiedApplyAuthorizationV1, VerifiedSecretSubjectV1,
    WorkerDependencySelectionChangeV1, WorkspaceId,
};

use super::LocalControlAdapter;

impl LocalControlAdapter {
    fn coordinator(&self) -> TransactionCoordinator<'_, Self, Self, Self, Self, Self> {
        TransactionCoordinator::new(self, self, self, self, self, &self.admission)
    }

    pub(super) fn apply_subscription_maintenance_prepared(
        &self,
        prepared: hiroute_application::PreparedTransactionV1,
    ) -> Result<OperationV1, TransactionError> {
        let principal = VerifiedPrincipal::for_subscription_maintenance();
        let accepted =
            self.coordinator()
                .accept_prepared(&WorkspaceId::default(), &principal, prepared)?;
        let operation = self.coordinator().run(&accepted.operation().operation_id)?;
        if operation.state == hiroute_domain::OperationState::Succeeded {
            self.finish_compute_subscription_save(&operation);
        }
        Ok(operation)
    }

    fn apply_prepared_with_principal(
        &self,
        principal: VerifiedPrincipal,
        prepared: hiroute_application::PreparedTransactionV1,
    ) -> Result<OperationV1, TransactionError> {
        let diagnostics = self
            .publication_diagnostics
            .lock()
            .map(|p| p.clone())
            .unwrap_or_default();
        let accepted = measure(
            &diagnostics,
            PublicationStage::Admission,
            None,
            None,
            || {
                self.coordinator()
                    .accept_prepared(&WorkspaceId::default(), &principal, prepared)
            },
        )?;
        let operation_id = accepted.operation().operation_id.to_string();
        let operation = measure(
            &diagnostics,
            PublicationStage::Execute,
            Some(&operation_id),
            None,
            || self.coordinator().run_accepted(accepted),
        )?;
        if operation.state == hiroute_domain::OperationState::Succeeded {
            self.finish_compute_subscription_save(&operation);
            crate::publication_failpoint::crash("after_terminal");
        }
        Ok(operation)
    }

    pub(super) fn reconcile_startup_and_open(&self) -> Result<(), String> {
        self.coordinator()
            .reconcile_startup_and_open()
            .and_then(|_| {
                self.reconcile_plan_content_heads()
                    .map_err(TransactionError::Port)
            })
            .map_err(|error| {
                let owner = self
                    .stores_lock()
                    .ok()
                    .and_then(|stores| stores.control().writer_claim_operation().ok().flatten());
                match owner {
                    Some(operation) => {
                        let code = operation
                            .safe_error_code
                            .as_deref()
                            .filter(|code| {
                                code.len() <= 64
                                    && code
                                        .bytes()
                                        .all(|byte| byte.is_ascii_uppercase() || byte == b'_')
                            })
                            .unwrap_or("RECOVERY_REQUIRED");
                        format!(
                            "{error}; operation_id={} state={} code={code}",
                            operation.operation_id,
                            operation.state.as_str()
                        )
                    }
                    None => error.to_string(),
                }
            })
    }

    pub(super) fn refresh_discovery(
        &self,
    ) -> Result<Vec<hiroute_integrations::FilesystemAgentDiscoveryV1>, String> {
        let discoveries = self.scanner.scan();
        let findings = discoveries
            .iter()
            .filter_map(|discovery| discovery.permission_hardening.clone())
            .map(|finding| (finding.discovered_source_ref.clone(), finding))
            .collect::<std::collections::BTreeMap<_, _>>();
        let mut inputs = self
            .protected_inputs
            .lock()
            .map_err(|_| "protected discovery registry is unavailable".to_owned())?;
        inputs.clear();
        for descriptor in discoveries
            .iter()
            .filter_map(|discovery| discovery.discovered_credential.clone())
        {
            let slot = protected_input_slot(&descriptor)?;
            if inputs.insert(slot, descriptor).is_some() {
                return Err("protected discovery slot collision".to_owned());
            }
        }
        *self
            .permission_findings
            .lock()
            .map_err(|_| "permission discovery registry is unavailable".to_owned())? = findings;
        Ok(discoveries)
    }
}

pub(super) fn protected_input_slot(
    descriptor: &hiroute_integrations::DiscoveredCredentialRefV1,
) -> Result<String, String> {
    let digest = CanonicalDigest::of(descriptor).map_err(|error| error.to_string())?;
    Ok(format!("discovered.{}", &digest.as_str()[7..55]))
}

impl ApplicationMutationPort for LocalControlAdapter {
    fn preview_change(
        &self,
        request: PreviewRequestV1,
    ) -> Result<PreviewResultV1, TransactionError> {
        self.coordinator().preview(&WorkspaceId::default(), request)
    }

    fn apply_change(
        &self,
        principal_kind: PrincipalKind,
        request: ApplyRequestV1,
    ) -> Result<OperationV1, TransactionError> {
        let principal = VerifiedPrincipal::from_protected_launcher(principal_kind)?;
        let accepted = self
            .coordinator()
            .accept(&WorkspaceId::default(), &principal, request)?;
        let operation = self.coordinator().run_accepted(accepted)?;
        if operation.state == hiroute_domain::OperationState::Succeeded {
            self.finish_compute_subscription_save(&operation);
            crate::publication_failpoint::crash("after_terminal");
        }
        Ok(operation)
    }

    fn apply_local_change(&self, request: ApplyRequestV1) -> Result<OperationV1, TransactionError> {
        let accepted = self.coordinator().accept(
            &WorkspaceId::default(),
            &VerifiedPrincipal::for_local_control(),
            request,
        )?;
        let operation = self.coordinator().run_accepted(accepted)?;
        if operation.state == hiroute_domain::OperationState::Succeeded {
            self.finish_compute_subscription_save(&operation);
            crate::publication_failpoint::crash("after_terminal");
        }
        Ok(operation)
    }

    fn apply_prepared_change(
        &self,
        principal_kind: PrincipalKind,
        prepared: hiroute_application::PreparedTransactionV1,
    ) -> Result<OperationV1, TransactionError> {
        let principal = VerifiedPrincipal::from_protected_launcher(principal_kind)?;
        self.apply_prepared_with_principal(principal, prepared)
    }

    fn apply_local_prepared_change(
        &self,
        prepared: hiroute_application::PreparedTransactionV1,
    ) -> Result<OperationV1, TransactionError> {
        self.apply_prepared_with_principal(VerifiedPrincipal::for_local_control(), prepared)
    }
}

impl ControlRepositoryPort for LocalControlAdapter {
    fn current_revisions(&self, workspace: &WorkspaceId) -> PortResult<RevisionSetV1> {
        self.stores_lock()?.control().current_revisions(workspace)
    }

    fn operation_for_idempotency(
        &self,
        workspace: &WorkspaceId,
        scope: &IdempotencyScopeV1,
    ) -> PortResult<Option<OperationV1>> {
        self.stores_lock()?
            .control()
            .operation_for_idempotency(workspace, scope)
    }

    fn begin_local_operation(&self, operation: &OperationV1) -> PortResult<BeginOperationOutcome> {
        self.stores_lock()?
            .control()
            .begin_local_operation(operation)
    }

    fn verify_apply_authorization(
        &self,
        capability: &ProtectedApplyCapability,
        workspace: &WorkspaceId,
        principal: &str,
        operation_kind: &str,
        accepted_digest: &CanonicalDigest,
        expected_revisions: &RevisionSetV1,
    ) -> PortResult<VerifiedApplyAuthorizationV1> {
        self.stores_lock()?.control().verify_apply_authorization(
            capability,
            workspace,
            principal,
            operation_kind,
            accepted_digest,
            expected_revisions,
        )
    }

    fn begin_operation(
        &self,
        operation: &OperationV1,
        authorization: &VerifiedApplyAuthorizationV1,
    ) -> PortResult<BeginOperationOutcome> {
        self.stores_lock()?
            .control()
            .begin_operation(operation, authorization)
    }

    fn operation_is_current(&self, operation: &OperationV1) -> PortResult<bool> {
        self.stores_lock()?
            .control()
            .operation_is_current(operation)
    }

    fn load_operation(&self, operation_id: &OperationId) -> PortResult<Option<OperationV1>> {
        self.stores_lock()?.control().load_operation(operation_id)
    }

    fn recoverable_operations(&self) -> PortResult<Vec<OperationV1>> {
        self.stores_lock()?.control().recoverable_operations()
    }

    fn writer_recovery_required(&self) -> PortResult<bool> {
        self.stores_lock()?.control().writer_recovery_required()
    }

    fn save_operation(&self, operation: &mut OperationV1) -> PortResult<()> {
        self.stores_lock()?.control().save_operation(operation)
    }

    fn worker_dependency_selection_revision(
        &self,
        workspace: &WorkspaceId,
        harness: hiroute_domain::delegation::WorkerHarnessV1,
    ) -> PortResult<u64> {
        self.stores_lock()?
            .control()
            .worker_dependency_selection(workspace, harness)
            .map(|selection| selection.map(|(_, revision)| revision).unwrap_or(0))
    }

    fn apply_worker_dependency_selection(
        &self,
        operation_id: &OperationId,
        workspace: &WorkspaceId,
        change: &WorkerDependencySelectionChangeV1,
    ) -> PortResult<OwnedEffectV1> {
        self.stores_lock()?
            .control()
            .apply_worker_dependency_selection(operation_id, workspace, change)
    }

    fn apply_control(
        &self,
        operation_id: &OperationId,
        workspace: &WorkspaceId,
        expected_revision: u64,
        desired: &serde_json::Value,
    ) -> PortResult<OwnedEffectV1> {
        self.stores_lock()?.control().apply_control(
            operation_id,
            workspace,
            expected_revision,
            desired,
        )
    }

    fn observe_control(
        &self,
        operation_id: &OperationId,
        workspace: &WorkspaceId,
    ) -> PortResult<EffectReconciliation> {
        self.stores_lock()?
            .control()
            .observe_control(operation_id, workspace)
    }

    fn activate_control(&self, effect: &OwnedEffectV1) -> PortResult<OwnedEffectV1> {
        self.stores_lock()?.control().activate_control(effect)
    }

    fn compensate_control(&self, effect: &OwnedEffectV1) -> PortResult<CompensationOutcome> {
        self.stores_lock()?.control().compensate_control(effect)
    }

    fn finish_operation(&self, operation: &mut OperationV1) -> PortResult<u64> {
        self.finish_settings_collaboration(operation)?;
        self.stores_lock()?.control().finish_operation(operation)
    }

    fn save_operation_tail(&self, operation: &mut OperationV1) -> PortResult<()> {
        self.stores_lock()?.control().save_operation_tail(operation)
    }

    fn reclaim_operation_writer(&self, operation_id: &OperationId) -> PortResult<()> {
        self.stores_lock()?
            .control()
            .reclaim_operation_writer(operation_id)
    }

    fn save_agent_surface_check(
        &self,
        workspace: &WorkspaceId,
        record: &hiroute_domain::AgentSurfaceCheckRecordV1,
    ) -> PortResult<bool> {
        self.stores_lock()?
            .control()
            .save_agent_surface_check(workspace, record)
    }

    fn agent_surface_checks(
        &self,
        workspace: &WorkspaceId,
        context_id: &str,
    ) -> PortResult<Vec<hiroute_domain::AgentSurfaceCheckRecordV1>> {
        self.stores_lock()?
            .control()
            .agent_surface_checks(workspace, context_id)
    }
}

impl ComputeSourceControlPort for LocalControlAdapter {
    fn apply_compute_source(
        &self,
        operation_id: &OperationId,
        workspace: &WorkspaceId,
        expected_target_revision: u64,
        mutation: &ComputeSourceMutationV1,
    ) -> PortResult<OwnedEffectV1> {
        self.stores_lock()?.control().apply_compute_source(
            operation_id,
            workspace,
            expected_target_revision,
            mutation,
        )
    }
}

impl CredentialPoolControlPort for LocalControlAdapter {
    fn apply_credential_pool(
        &self,
        operation_id: &OperationId,
        workspace: &WorkspaceId,
        expected_target_revision: u64,
        mutation: &CredentialPoolMutationV1,
    ) -> PortResult<OwnedEffectV1> {
        self.stores_lock()?.control().apply_credential_pool(
            operation_id,
            workspace,
            expected_target_revision,
            mutation,
        )
    }
}

impl SecretStorePort for LocalControlAdapter {
    fn generation(&self, credential: &CredentialRefV1) -> PortResult<u64> {
        self.stores_lock()?.secrets().generation(credential)
    }

    fn fingerprint(&self, secret: &ProtectedSecret) -> PortResult<CanonicalDigest> {
        self.stores_lock()?.secrets().fingerprint(secret)
    }

    fn resolve_secret(
        &self,
        subject: &VerifiedSecretSubjectV1,
        credential: &CredentialRefV1,
        purpose: &str,
        destination: &str,
        expected_generation: u64,
    ) -> PortResult<ProtectedSecret> {
        self.stores_lock()?.secrets().resolve_secret(
            subject,
            credential,
            purpose,
            destination,
            expected_generation,
        )
    }

    fn apply_secret(
        &self,
        operation_id: &OperationId,
        mutation: &SecretMutationV1,
        input: Option<&ProtectedSecret>,
    ) -> PortResult<OwnedEffectV1> {
        self.stores_lock()?
            .secrets()
            .apply_secret(operation_id, mutation, input)
    }

    fn observe_secret(
        &self,
        operation_id: &OperationId,
        mutation: &SecretMutationV1,
    ) -> PortResult<EffectReconciliation> {
        self.stores_lock()?
            .secrets()
            .observe_secret(operation_id, mutation)
    }

    fn activate_secret(&self, effect: &OwnedEffectV1) -> PortResult<OwnedEffectV1> {
        self.stores_lock()?.secrets().activate_secret(effect)
    }

    fn compensate_secret(&self, effect: &OwnedEffectV1) -> PortResult<CompensationOutcome> {
        self.stores_lock()?.secrets().compensate_secret(effect)
    }

    fn inspect_agent_access_grant(
        &self,
        owner_scope: &str,
        connection_id: &str,
    ) -> PortResult<Option<AgentAccessGrantRefV1>> {
        self.stores_lock()?
            .secrets()
            .inspect_agent_access_grant(owner_scope, connection_id)
    }

    fn resolve_agent_access_grant(
        &self,
        reference: &AgentAccessGrantRefV1,
    ) -> PortResult<AgentAccessGrantMaterial> {
        self.stores_lock()?
            .secrets()
            .resolve_agent_access_grant(reference)
    }

    fn apply_agent_access_grant(
        &self,
        operation_id: &OperationId,
        mutation: &AgentAccessGrantMutationV1,
        input: Option<&ProtectedSecret>,
    ) -> PortResult<OwnedEffectV1> {
        self.stores_lock()?
            .secrets()
            .apply_agent_access_grant(operation_id, mutation, input)
    }

    fn observe_agent_access_grant(
        &self,
        operation_id: &OperationId,
        mutation: &AgentAccessGrantMutationV1,
    ) -> PortResult<EffectReconciliation> {
        self.stores_lock()?
            .secrets()
            .observe_agent_access_grant(operation_id, mutation)
    }

    fn activate_agent_access_grant(&self, effect: &OwnedEffectV1) -> PortResult<OwnedEffectV1> {
        self.stores_lock()?
            .secrets()
            .activate_agent_access_grant(effect)
    }

    fn compensate_agent_access_grant(
        &self,
        effect: &OwnedEffectV1,
    ) -> PortResult<CompensationOutcome> {
        self.stores_lock()?
            .secrets()
            .compensate_agent_access_grant(effect)
    }
}

impl RuntimeStatePort for LocalControlAdapter {
    fn generation(&self, key: &str) -> PortResult<u64> {
        self.stores_lock()?.runtime().generation(key)
    }

    fn apply_runtime(
        &self,
        operation_id: &OperationId,
        mutation: &RuntimeMutationV1,
    ) -> PortResult<OwnedEffectV1> {
        self.stores_lock()?
            .runtime()
            .apply_runtime(operation_id, mutation)
    }

    fn observe_runtime(
        &self,
        operation_id: &OperationId,
        mutation: &RuntimeMutationV1,
    ) -> PortResult<EffectReconciliation> {
        self.stores_lock()?
            .runtime()
            .observe_runtime(operation_id, mutation)
    }

    fn activate_runtime(&self, effect: &OwnedEffectV1) -> PortResult<OwnedEffectV1> {
        self.stores_lock()?.runtime().activate_runtime(effect)
    }

    fn compensate_runtime(&self, effect: &OwnedEffectV1) -> PortResult<CompensationOutcome> {
        self.stores_lock()?.runtime().compensate_runtime(effect)
    }
}

impl ProtectedInputPort for LocalControlAdapter {
    fn read_secret(&self, input_slot: &str) -> PortResult<ProtectedSecret> {
        if let Some(secret) = self
            .agent_token_inputs
            .lock()
            .map_err(|_| unavailable("agent_token.input.lock"))?
            .get(input_slot)
        {
            return ProtectedSecret::new(secret.expose().to_vec())
                .map_err(|_| unavailable("agent_token.input.invalid"));
        }
        if let Some(secret) = self
            .manual_protected_inputs
            .lock()
            .map_err(|_| unavailable("manual.input.lock"))?
            .get(input_slot)
        {
            return ProtectedSecret::new(secret.expose().to_vec())
                .map_err(|_| unavailable("manual.input.invalid"));
        }
        let descriptor = self
            .protected_inputs
            .lock()
            .map_err(|_| unavailable("discovery.input.lock"))?
            .get(input_slot)
            .cloned()
            .ok_or_else(|| PortError::new(PortErrorCode::NotFound, "discovery.input.missing"))?;
        self.scanner
            .read_discovered_secret(&descriptor)
            .map_err(|_| PortError::new(PortErrorCode::Conflict, "discovery.input.changed"))
    }

    fn validate_discovery_evidence(
        &self,
        input_slot: &str,
        expected_evidence: &CanonicalDigest,
    ) -> PortResult<()> {
        self.validate_compute_discovery_evidence(input_slot, expected_evidence)
            .map_err(|_| {
                PortError::new(PortErrorCode::Conflict, "discovery.input.evidence_changed")
            })
    }
}

impl LocalControlAdapter {
    pub(super) fn stores_lock(
        &self,
    ) -> PortResult<std::sync::MutexGuard<'_, hiroute_local_storage::LocalStorageSet>> {
        self.stores.lock().map_err(|_| unavailable("storage.lock"))
    }
}

fn unavailable(context: &'static str) -> PortError {
    PortError::new(PortErrorCode::Unavailable, context)
}
