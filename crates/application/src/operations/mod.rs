use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

use hiroute_application_api::{
    ApplyRequestV1, ErrorCode, PreviewRequestV1, PreviewResultV1, PrincipalKind, command_by_id,
};
use hiroute_domain::{
    AgentAccessGrantMutationKindV1, CanonicalDigest, CompensationOutcome, ComputeSourceControlPort,
    ControlRepositoryPort, CredentialPoolControlPort, EffectReconciliation, ExternalEffectPort,
    OperationId, OperationState, OperationStepKind, OperationStepStatus, OperationV1,
    OwnedEffectKind, OwnedEffectV1, PortError, RevisionMismatch, RuntimeStatePort,
    SecretMutationKind, SecretStorePort, WorkspaceId,
};
use serde::Serialize;
use thiserror::Error;

use crate::change::{
    ChangePreparationError, ConnectionOptionAuthorizationPort, PreparedPreview, ProtectedInputPort,
    prepare_preview,
};

mod agent_access_grants;
mod compute_compensation;
mod execution;
mod prepared;

pub use prepared::PreparedTransactionV1;

pub(crate) mod control;

pub struct AcceptedApply {
    operation: OperationV1,
    existing: bool,
}

impl AcceptedApply {
    pub fn operation(&self) -> &OperationV1 {
        &self.operation
    }

    pub const fn existing(&self) -> bool {
        self.existing
    }
}

/// Principal identity established by authenticated local transport. This sealed type is not
/// serde/wire constructible and is the only identity accepted by the transaction engine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedPrincipal {
    scope: String,
    local_control: bool,
}

impl VerifiedPrincipal {
    pub fn from_protected_launcher(kind: PrincipalKind) -> Result<Self, TransactionError> {
        let scope = match kind {
            PrincipalKind::InteractiveUser => "interactive-user",
            PrincipalKind::Desktop => "desktop",
            PrincipalKind::Skill | PrincipalKind::SealedCollaboration => {
                return Err(TransactionError::CapabilityDenied);
            }
        };
        Ok(Self {
            scope: scope.to_owned(),
            local_control: false,
        })
    }

    /// Process-local authority for the bounded maintenance of an already confirmed subscription.
    /// It has no wire representation and is accepted only with an exact one-shot capability.
    pub fn for_subscription_maintenance() -> Self {
        Self {
            scope: "daemon-subscription-maintenance".to_owned(),
            local_control: false,
        }
    }

    /// Identity established by the owner-only Local Control socket and peer UID check. It has no
    /// wire token and is accepted only by explicitly Released local mutation paths.
    pub fn for_local_control() -> Self {
        Self {
            scope: "interactive-user".to_owned(),
            local_control: true,
        }
    }
}

/// One per hirouted process. It starts closed, serializes every writer, and opens only after all
/// durable admitted Operations have been reconciled.
#[derive(Debug, Default)]
pub struct TransactionRuntime {
    writer: Mutex<()>,
    writes_open: AtomicBool,
}

impl TransactionRuntime {
    fn lock_writer(&self) -> Result<MutexGuard<'_, ()>, TransactionError> {
        self.writer
            .lock()
            .map_err(|_| TransactionError::RecoveryRequired)
    }
}

pub struct TransactionCoordinator<'a, C, S, R, E, I> {
    control: &'a C,
    secrets: &'a S,
    runtime: &'a R,
    external: &'a E,
    protected_inputs: &'a I,
    admission: &'a TransactionRuntime,
}

impl<'a, C, S, R, E, I> TransactionCoordinator<'a, C, S, R, E, I>
where
    C: ControlRepositoryPort
        + ComputeSourceControlPort
        + CredentialPoolControlPort
        + ConnectionOptionAuthorizationPort,
    S: SecretStorePort,
    R: RuntimeStatePort,
    E: ExternalEffectPort,
    I: ProtectedInputPort,
{
    pub fn new(
        control: &'a C,
        secrets: &'a S,
        runtime: &'a R,
        external: &'a E,
        protected_inputs: &'a I,
        admission: &'a TransactionRuntime,
    ) -> Self {
        Self {
            control,
            secrets,
            runtime,
            external,
            protected_inputs,
            admission,
        }
    }

    pub fn reconcile_startup_and_open(&self) -> Result<Vec<OperationV1>, TransactionError> {
        let _writer = self.admission.lock_writer()?;
        self.admission.writes_open.store(false, Ordering::Release);
        let operations = self.control.recoverable_operations()?;
        let mut recovered = Vec::with_capacity(operations.len());
        for operation in operations {
            let terminal = self.run_current_or_reload_locked(operation, false)?;
            if terminal.state == OperationState::NeedsAttention {
                return Err(TransactionError::RecoveryRequired);
            }
            recovered.push(terminal);
        }
        if self.control.writer_recovery_required()? {
            return Err(TransactionError::RecoveryRequired);
        }
        self.admission.writes_open.store(true, Ordering::Release);
        Ok(recovered)
    }

    pub fn preview(
        &self,
        workspace: &WorkspaceId,
        request: PreviewRequestV1,
    ) -> Result<PreviewResultV1, TransactionError> {
        if hiroute_domain::CHANGE_SPEC_SCHEMA_V1
            .negotiate(request.schema_version)
            .is_none()
        {
            return Err(TransactionError::InvalidArguments);
        }
        let revisions = self.control.current_revisions(workspace)?;
        Ok(prepare_preview(
            self.secrets,
            self.runtime,
            self.external,
            self.protected_inputs,
            self.control,
            request,
            revisions,
        )?
        .result)
    }

    /// Applies the exact ordering contract: lookup idempotency first, then target/dependency
    /// revisions, then normalization/digest, and only then the first durable write.
    pub fn accept(
        &self,
        workspace: &WorkspaceId,
        principal: &VerifiedPrincipal,
        request: ApplyRequestV1,
    ) -> Result<AcceptedApply, TransactionError> {
        if hiroute_domain::CHANGE_SPEC_SCHEMA_V1
            .negotiate(request.schema_version)
            .is_none()
        {
            return Err(TransactionError::InvalidArguments);
        }
        let descriptor = command_by_id(&request.spec.command_id)
            .filter(|descriptor| descriptor.kind == hiroute_application_api::CommandKind::Apply)
            .ok_or(TransactionError::InvalidArguments)?;
        self.accept_with_reproduced_plan(
            workspace,
            principal,
            &descriptor.operation_id,
            request,
            |current, request| {
                let PreparedPreview { result, plan } = prepare_preview(
                    self.secrets,
                    self.runtime,
                    self.external,
                    self.protected_inputs,
                    self.control,
                    PreviewRequestV1::new(request.spec.clone()),
                    current.clone(),
                )?;
                for intent in plan.external() {
                    self.external.validate_external_admission(intent)?;
                }
                Ok((result.change_digest, plan))
            },
        )
    }

    /// Consume the sealed admission result. A concurrent writer forces a fresh durable read.
    pub fn run_accepted(&self, accepted: AcceptedApply) -> Result<OperationV1, TransactionError> {
        let _writer = self.admission.lock_writer()?;
        self.run_current_or_reload_locked(accepted.operation, true)
    }

    fn run_current_or_reload_locked(
        &self,
        operation: OperationV1,
        retry_tail: bool,
    ) -> Result<OperationV1, TransactionError> {
        if self.control.operation_is_current(&operation)? {
            self.run_operation_locked(operation, retry_tail)
        } else {
            self.run_locked(&operation.operation_id, retry_tail)
        }
    }

    pub fn run(&self, operation_id: &OperationId) -> Result<OperationV1, TransactionError> {
        let _writer = self.admission.lock_writer()?;
        self.run_locked(operation_id, true)
    }

    fn run_locked(
        &self,
        operation_id: &OperationId,
        retry_tail: bool,
    ) -> Result<OperationV1, TransactionError> {
        let operation = self
            .control
            .load_operation(operation_id)?
            .ok_or(TransactionError::OperationNotFound)?;
        self.run_operation_locked(operation, retry_tail)
    }

    fn run_operation_locked(
        &self,
        mut operation: OperationV1,
        retry_tail: bool,
    ) -> Result<OperationV1, TransactionError> {
        if operation.state == OperationState::NeedsAttention {
            // A previous build may have parked after every effect was compensated but before
            // the terminal proof completed. Recheck exact ownership; never release the writer
            // for an effect whose compensation remains uncertain.
            if operation
                .steps
                .iter()
                .all(|step| step.status == OperationStepStatus::Compensated)
                && self.reconcile_terminal_rollback(&operation).is_ok()
            {
                operation.transition(OperationState::RolledBack)?;
                self.control.finish_operation(&mut operation)?;
                self.external.finish_publication_activation(&operation)?;
            }
            return Ok(operation);
        }
        if operation.state.is_terminal() {
            return Ok(operation);
        }
        if operation.state == OperationState::RollingBack {
            return self.finish_rollback(operation, "RECOVERY_ROLLBACK");
        }
        if let Some(receipt) = settings_service_receipt(&operation) {
            return self.resume_settings_tail(operation, &receipt, retry_tail);
        }

        for kind in OperationStepKind::ALL {
            if operation.step(kind).status == OperationStepStatus::Applied
                && kind != OperationStepKind::Activate
            {
                continue;
            }
            if let Err(error) = self.run_step(&mut operation, kind) {
                if let Some(receipt) = settings_service_receipt(&operation)
                    && matches!(
                        settings_service_completion_digest(
                            &operation,
                            receipt.publication_revision,
                            &receipt.publication_digest,
                        ),
                        Ok(expected) if expected == receipt.completed_effects_digest
                    )
                {
                    // The service segment is durably complete; only the client file tail
                    // remains, so parking must never roll the publication back.
                    operation.safe_error_code = Some(error.safe_code().to_owned());
                    self.control.save_operation_tail(&mut operation)?;
                    return Ok(operation);
                }
                let code = error.safe_code();
                return self.finish_rollback(operation, code);
            }
        }
        self.finish_success(operation)
    }

    fn finish_success(&self, mut operation: OperationV1) -> Result<OperationV1, TransactionError> {
        if let Err(error) = self.reconcile_terminal_success(&mut operation) {
            let code = error.safe_code();
            return self.finish_rollback(operation, code);
        }
        let is_price_change = operation.plan.source_price_change().is_some();
        if is_price_change || operation.plan.compute_source().is_some() {
            match self.external.install_source_price_snapshot(&operation) {
                Ok(generation) if is_price_change => {
                    operation
                        .step_mut(OperationStepKind::Activate)
                        .terminal_result = Some(
                        serde_json::to_string(&generation)
                            .map_err(|_| TransactionError::InvalidArguments)?,
                    );
                    operation.safe_error_code = None;
                    self.control.save_operation(&mut operation)?;
                }
                Err(_) if is_price_change => {
                    // Configuration has committed; do not roll it back or accept a newer writer
                    // while this generation still needs installation. Restart resumes this step.
                    self.admission.writes_open.store(false, Ordering::Release);
                    operation.safe_error_code = Some("PRICE_RECOVERY_REQUIRED".into());
                    self.control.save_operation(&mut operation)?;
                    return Err(TransactionError::RecoveryRequired);
                }
                _ => (),
            }
        }
        operation.transition(OperationState::Succeeded)?;
        self.control.finish_operation(&mut operation)?;
        self.external.finish_publication_activation(&operation)?;
        Ok(operation)
    }

    /// A settings operation whose service segment is sealed by a completion receipt. The
    /// client-file tail is retried only on an explicit run; startup reports it untouched.
    fn resume_settings_tail(
        &self,
        operation: OperationV1,
        receipt: &hiroute_domain::SettingsServiceCompletionV1,
        retry_tail: bool,
    ) -> Result<OperationV1, TransactionError> {
        if !matches!(
            settings_service_completion_digest(
                &operation,
                receipt.publication_revision,
                &receipt.publication_digest,
            ),
            Ok(expected) if expected == receipt.completed_effects_digest
        ) {
            return self.finish_rollback(operation, "SETTINGS_RECEIPT_INVALID");
        }
        for intent in operation
            .plan
            .external()
            .iter()
            .filter(|intent| intent.kind() == OwnedEffectKind::Publication)
        {
            match self.external.observe_external(&operation, intent)? {
                EffectReconciliation::Applied(_) => {}
                // The receipt cannot prove the service segment; the generic path re-verifies it.
                EffectReconciliation::Staged(_) | EffectReconciliation::Missing => {
                    let mut operation = operation;
                    operation
                        .step_mut(OperationStepKind::Activate)
                        .terminal_result = None;
                    self.control.save_operation(&mut operation)?;
                    return self.run_steps_and_finish(operation);
                }
                // A later publication replaced this tail's root: expired, never rewrite files.
                EffectReconciliation::OwnershipLost(_) => {
                    let mut operation = operation;
                    operation.safe_error_code = Some("SETTINGS_TAIL_EXPIRED".into());
                    self.control.save_operation_tail(&mut operation)?;
                    return Ok(operation);
                }
            }
        }
        let files_applied = operation
            .plan
            .external()
            .iter()
            .filter(|intent| hiroute_domain::is_settings_managed_configuration(intent))
            .all(|intent| {
                matches!(
                    self.external.observe_external(&operation, intent),
                    Ok(EffectReconciliation::Applied(_))
                )
            });
        if files_applied {
            // The files were already switched; complete the terminal record, never rewrite.
            return self.run_steps_and_finish(operation);
        }
        if !retry_tail {
            // Startup only reports a pending tail: no file writes and no writer retention. The
            // generic adapter code cannot express "service done, file tail awaiting retry".
            let mut operation = operation;
            operation.safe_error_code = Some("SETTINGS_TAIL_PENDING".into());
            self.control.save_operation_tail(&mut operation)?;
            return Ok(operation);
        }
        // Explicit same-operation retry: re-acquire the claim, then only the file tail runs.
        self.control
            .reclaim_operation_writer(&operation.operation_id)?;
        let mut operation = self
            .control
            .load_operation(&operation.operation_id)?
            .ok_or(TransactionError::OperationNotFound)?;
        match self.activate_settings_file_tail(&mut operation) {
            Ok(()) => {
                // The explicit tail retry bypasses run_step; complete its step journal before
                // publishing terminal success so strict history readers can verify the result.
                operation.step_mut(OperationStepKind::Activate).status =
                    OperationStepStatus::Applied;
                operation.safe_error_code = None;
                self.control.save_operation(&mut operation)?;
                self.finish_success(operation)
            }
            Err(error) => {
                operation.safe_error_code = Some(error.safe_code().to_owned());
                self.control.save_operation_tail(&mut operation)?;
                Ok(operation)
            }
        }
    }

    fn run_steps_and_finish(
        &self,
        mut operation: OperationV1,
    ) -> Result<OperationV1, TransactionError> {
        for kind in OperationStepKind::ALL {
            if operation.step(kind).status == OperationStepStatus::Applied
                && kind != OperationStepKind::Activate
            {
                continue;
            }
            if let Err(error) = self.run_step(&mut operation, kind) {
                let code = error.safe_code();
                return self.finish_rollback(operation, code);
            }
        }
        self.finish_success(operation)
    }

    pub fn recover_all(&self) -> Vec<Result<OperationV1, TransactionError>> {
        match self.control.recoverable_operations() {
            Ok(operations) => operations
                .into_iter()
                .map(|operation| {
                    let _writer = self.admission.lock_writer()?;
                    self.run_current_or_reload_locked(operation, true)
                })
                .collect(),
            Err(error) => vec![Err(TransactionError::Port(error))],
        }
    }

    /// A step journal saying `Applied` is not sufficient evidence for terminal success. Every
    /// exact-owned adapter marker and committed view is re-authenticated at the terminal boundary.
    fn reconcile_terminal_success(
        &self,
        operation: &mut OperationV1,
    ) -> Result<(), TransactionError> {
        for mutation in operation.plan.secrets().to_vec() {
            require_applied(
                self.secrets
                    .observe_secret(&operation.operation_id, &mutation)?,
            )?;
        }
        self.require_agent_access_grants_applied(operation)?;
        require_applied(
            self.control
                .observe_control(&operation.operation_id, &operation.workspace_id)?,
        )?;
        for intent in operation.plan.external().to_vec() {
            require_applied(self.external.observe_external(operation, &intent)?)?;
        }
        for mutation in operation.plan.runtime().to_vec() {
            require_applied(
                self.runtime
                    .observe_runtime(&operation.operation_id, &mutation)?,
            )?;
        }
        Ok(())
    }

    fn record_effect(
        &self,
        operation: &mut OperationV1,
        step: OperationStepKind,
        effect: OwnedEffectV1,
    ) -> Result<(), TransactionError> {
        let journal = operation.step_mut(step);
        if let Some(existing) = journal
            .effects
            .iter_mut()
            .find(|existing| existing.effect_id == effect.effect_id)
        {
            if *existing == effect {
                return Ok(());
            }
            *existing = effect;
        } else {
            journal.effects.push(effect);
        }
        self.control.save_operation(operation)?;
        Ok(())
    }

    fn finish_rollback(
        &self,
        mut operation: OperationV1,
        safe_error_code: &str,
    ) -> Result<OperationV1, TransactionError> {
        // A publication may have crossed a durable boundary before its adapter returned.
        // Obtain and persist the abort decision before compensating ANY dependent effect.
        for effect in operation
            .step(OperationStepKind::CompilePublication)
            .effects
            .clone()
        {
            match self
                .external
                .prepare_publication_rollback(&operation, &effect)
            {
                Ok(checkpoint) => self.record_effect(
                    &mut operation,
                    OperationStepKind::CompilePublication,
                    checkpoint,
                )?,
                Err(error) if error.code == hiroute_domain::PortErrorCode::Unavailable => {
                    self.admission.writes_open.store(false, Ordering::Release);
                    operation.safe_error_code = Some("PUBLICATION_RECOVERY_REQUIRED".to_owned());
                    self.control.save_operation(&mut operation)?;
                    return Err(TransactionError::RecoveryRequired);
                }
                Err(_) => {
                    self.admission.writes_open.store(false, Ordering::Release);
                    if operation.state != OperationState::RollingBack {
                        operation.transition(OperationState::RollingBack)?;
                    }
                    operation.transition(OperationState::NeedsAttention)?;
                    operation.safe_error_code =
                        Some("PUBLICATION_OWNERSHIP_UNCONFIRMED".to_owned());
                    self.control.finish_operation(&mut operation)?;
                    return Ok(operation);
                }
            }
        }
        operation.safe_error_code = Some(safe_error_code.to_owned());
        if operation.state != OperationState::RollingBack {
            operation.transition(OperationState::RollingBack)?;
        }
        self.control.save_operation(&mut operation)?;

        let mut uncertain = false;
        for kind in OperationStepKind::ALL.into_iter().rev() {
            if operation.step(kind).status == OperationStepStatus::Compensated {
                continue;
            }
            operation.step_mut(kind).status = OperationStepStatus::Compensating;
            self.control.save_operation(&mut operation)?;
            let reconciliation = self.reconcile_step_for_rollback(&mut operation, kind);
            // Reconciliation uncertainty must not prevent best-effort compensation of the
            // other effects that still prove exact Operation ownership.
            let compensation = self.compensate_step(&operation, kind);
            let restored_stage_discarded = execution::is_settings_restoration(&operation)
                && kind == OperationStepKind::ApplyAgentArtifacts
                && compensation.is_ok()
                && operation
                    .plan
                    .external()
                    .iter()
                    .filter(|intent| intent.kind() == OwnedEffectKind::AgentArtifact)
                    .all(|intent| {
                        matches!(
                            self.external.observe_external(&operation, intent),
                            Ok(EffectReconciliation::Missing)
                        )
                    });
            let outcome = match (reconciliation, compensation) {
                (Ok(()), Ok(())) => Ok(()),
                (Err(_), Ok(())) if restored_stage_discarded => Ok(()),
                _ => Err(TransactionError::EffectOwnershipLost),
            };
            match outcome {
                Ok(()) => {
                    let step = operation.step_mut(kind);
                    step.status = OperationStepStatus::Compensated;
                    step.terminal_result = Some("compensated".to_owned());
                }
                Err(_) => {
                    uncertain = true;
                    let step = operation.step_mut(kind);
                    step.status = OperationStepStatus::Attention;
                    step.terminal_result = Some("ownership_unconfirmed".to_owned());
                }
            }
            self.control.save_operation(&mut operation)?;
        }

        if !uncertain && self.reconcile_terminal_rollback(&operation).is_err() {
            uncertain = true;
        }
        if uncertain {
            operation.transition(OperationState::NeedsAttention)?;
        } else {
            operation.transition(OperationState::RolledBack)?;
        }
        self.control.finish_operation(&mut operation)?;
        self.external.finish_publication_activation(&operation)?;
        Ok(operation)
    }

    fn reconcile_terminal_rollback(&self, operation: &OperationV1) -> Result<(), TransactionError> {
        // Re-run every recorded compensation by exact ownership token. `AlreadyCompensated` is
        // positive durable evidence; a vanished/corrupt marker is uncertainty, not proof that the
        // effect never existed.
        for kind in OperationStepKind::ALL.into_iter().rev() {
            for effect in operation.step(kind).effects.iter().rev() {
                require_compensated(self.compensate_effect(operation, effect)?)?;
            }
        }
        for mutation in operation.plan.secrets() {
            require_missing(
                self.secrets
                    .observe_secret(&operation.operation_id, mutation)?,
            )?;
        }
        self.require_agent_access_grants_missing(operation)?;
        require_missing(
            self.control
                .observe_control(&operation.operation_id, &operation.workspace_id)?,
        )?;
        for intent in operation.plan.external() {
            if intent.kind() == OwnedEffectKind::LoginItem {
                // Desktop performs and, on a definitive rollback, compensates this action.
                // The daemon's observation is the host's original declaration, so it stays
                // "Applied" even after every daemon-owned effect is gone.
                continue;
            }
            require_missing(self.external.observe_external(operation, intent)?)?;
        }
        for mutation in operation.plan.runtime() {
            require_missing(
                self.runtime
                    .observe_runtime(&operation.operation_id, mutation)?,
            )?;
        }
        self.reconcile_compute_compensation(operation)?;
        Ok(())
    }

    fn reconcile_step_for_rollback(
        &self,
        operation: &mut OperationV1,
        kind: OperationStepKind,
    ) -> Result<(), TransactionError> {
        let mut ownership_lost = false;
        match kind {
            OperationStepKind::Prepare => {}
            OperationStepKind::ApplySecrets => {
                for mutation in operation.plan.secrets().to_vec() {
                    match self
                        .secrets
                        .observe_secret(&operation.operation_id, &mutation)?
                    {
                        EffectReconciliation::Missing => {}
                        EffectReconciliation::Staged(effect)
                        | EffectReconciliation::Applied(effect) => {
                            self.record_effect(operation, kind, effect)?;
                        }
                        EffectReconciliation::OwnershipLost(effect) => {
                            self.record_effect(operation, kind, effect)?;
                            ownership_lost = true;
                        }
                    }
                }
                if self
                    .reconcile_agent_access_grants_for_rollback(operation)
                    .is_err()
                {
                    ownership_lost = true;
                }
            }
            OperationStepKind::MaterializeSources => {
                match self
                    .control
                    .observe_control(&operation.operation_id, &operation.workspace_id)?
                {
                    EffectReconciliation::Missing => {}
                    EffectReconciliation::Staged(effect)
                    | EffectReconciliation::Applied(effect) => {
                        self.record_effect(operation, kind, effect)?;
                    }
                    EffectReconciliation::OwnershipLost(effect) => {
                        self.record_effect(operation, kind, effect)?;
                        ownership_lost = true;
                    }
                }
            }
            OperationStepKind::CompilePublication | OperationStepKind::ApplyAgentArtifacts => {
                let effect_kind = if kind == OperationStepKind::CompilePublication {
                    OwnedEffectKind::Publication
                } else {
                    OwnedEffectKind::AgentArtifact
                };
                let intents = operation
                    .plan
                    .external()
                    .iter()
                    .filter(|intent| intent.kind() == effect_kind)
                    .cloned()
                    .collect::<Vec<_>>();
                for intent in intents {
                    match self.external.observe_external(operation, &intent)? {
                        EffectReconciliation::Missing => {}
                        EffectReconciliation::Staged(effect)
                        | EffectReconciliation::Applied(effect) => {
                            self.record_effect(operation, kind, effect)?;
                        }
                        EffectReconciliation::OwnershipLost(effect) => {
                            self.record_effect(operation, kind, effect)?;
                            ownership_lost = true;
                        }
                    }
                }
            }
            OperationStepKind::Activate => {
                for mutation in operation.plan.runtime().to_vec() {
                    match self
                        .runtime
                        .observe_runtime(&operation.operation_id, &mutation)?
                    {
                        EffectReconciliation::Missing => {}
                        EffectReconciliation::Staged(effect)
                        | EffectReconciliation::Applied(effect) => {
                            self.record_effect(operation, kind, effect)?;
                        }
                        EffectReconciliation::OwnershipLost(effect) => {
                            self.record_effect(operation, kind, effect)?;
                            ownership_lost = true;
                        }
                    }
                }
            }
        }
        if ownership_lost {
            Err(TransactionError::EffectOwnershipLost)
        } else {
            Ok(())
        }
    }

    fn compensate_step(
        &self,
        operation: &OperationV1,
        kind: OperationStepKind,
    ) -> Result<(), TransactionError> {
        let mut uncertain = false;
        for effect in operation.step(kind).effects.iter().rev() {
            if self
                .compensate_effect(operation, effect)
                .and_then(require_compensated)
                .is_err()
            {
                uncertain = true;
            }
        }
        if uncertain {
            Err(TransactionError::EffectOwnershipLost)
        } else {
            Ok(())
        }
    }

    fn compensate_effect(
        &self,
        operation: &OperationV1,
        effect: &OwnedEffectV1,
    ) -> Result<CompensationOutcome, TransactionError> {
        Ok(match effect.kind {
            OwnedEffectKind::Control => self.control.compensate_control(effect)?,
            OwnedEffectKind::Secret if hiroute_domain::is_agent_access_grant_effect(effect) => {
                self.secrets.compensate_agent_access_grant(effect)?
            }
            OwnedEffectKind::Secret => self.secrets.compensate_secret(effect)?,
            OwnedEffectKind::RuntimeState => self.runtime.compensate_runtime(effect)?,
            OwnedEffectKind::Publication | OwnedEffectKind::AgentArtifact => {
                self.external.compensate_external(operation, effect)?
            }
            // The login item runs in the Desktop host under the native confirmation; the host
            // unregisters its own creation when the apply fails. Daemon-side rollback must
            // not touch the host's system registration, so compensation is deliberately empty.
            OwnedEffectKind::LoginItem => CompensationOutcome::AlreadyCompensated,
        })
    }
}

fn require_compensated(outcome: CompensationOutcome) -> Result<(), TransactionError> {
    match outcome {
        CompensationOutcome::Compensated | CompensationOutcome::AlreadyCompensated => Ok(()),
        CompensationOutcome::OwnershipLost => Err(TransactionError::EffectOwnershipLost),
    }
}

fn settings_service_receipt(
    operation: &OperationV1,
) -> Option<hiroute_domain::SettingsServiceCompletionV1> {
    operation
        .step(OperationStepKind::Activate)
        .terminal_result
        .as_deref()
        .and_then(hiroute_domain::SettingsServiceCompletionV1::parse)
}

/// Read readiness for a parked client-file tail. This does not authorize a new writer
/// or a file retry: the current publication must still match the sealed service proof.
pub fn settings_service_completion_is_current(
    operation: &OperationV1,
    publication_revision: u64,
    publication_digest: &CanonicalDigest,
) -> bool {
    operation.state == OperationState::Activating
        && operation.plan.spec().command_id == "agents.settings.apply"
        && operation
            .plan
            .external()
            .iter()
            .any(hiroute_domain::is_settings_managed_configuration)
        && operation.steps.iter().all(|step| {
            step.kind == OperationStepKind::Activate
                || step.status == OperationStepStatus::Applied
        })
        && settings_service_receipt(operation).is_some_and(|receipt| {
            receipt.publication_revision == publication_revision
                && receipt.publication_digest == *publication_digest
                && matches!(
                    settings_service_completion_digest(operation, publication_revision, publication_digest),
                    Ok(expected) if expected == receipt.completed_effects_digest
                )
        })
}

fn settings_service_completion_digest(
    operation: &OperationV1,
    publication_revision: u64,
    publication_digest: &CanonicalDigest,
) -> Result<CanonicalDigest, TransactionError> {
    CanonicalDigest::of(&(
        "hiroute.settings-service-proof/v2",
        operation.operation_id.as_str(),
        &operation.accepted_digest,
        publication_revision,
        publication_digest,
        operation
            .stable_input_digest()
            .map_err(|_| TransactionError::InvalidArguments)?,
    ))
    .map_err(|_| TransactionError::InvalidArguments)
}

fn require_applied(reconciliation: EffectReconciliation) -> Result<(), TransactionError> {
    match reconciliation {
        EffectReconciliation::Applied(_) => Ok(()),
        EffectReconciliation::Missing | EffectReconciliation::Staged(_) => {
            Err(TransactionError::EffectMissing)
        }
        EffectReconciliation::OwnershipLost(_) => Err(TransactionError::EffectOwnershipLost),
    }
}

fn require_missing(reconciliation: EffectReconciliation) -> Result<(), TransactionError> {
    match reconciliation {
        EffectReconciliation::Missing => Ok(()),
        EffectReconciliation::Staged(_)
        | EffectReconciliation::Applied(_)
        | EffectReconciliation::OwnershipLost(_) => Err(TransactionError::EffectOwnershipLost),
    }
}

fn apply_request_digest(request: &ApplyRequestV1) -> Result<CanonicalDigest, TransactionError> {
    #[derive(Serialize)]
    struct DigestInput<'a> {
        schema_version: hiroute_domain::SchemaVersion,
        spec: &'a hiroute_domain::ChangeSpecV1,
        accept_digest: &'a CanonicalDigest,
        expected_revisions: &'a hiroute_domain::RevisionSetV1,
        apply_capability: &'a Option<String>,
    }

    Ok(CanonicalDigest::of(&DigestInput {
        schema_version: request.schema_version,
        spec: &request.spec,
        accept_digest: &request.accept_digest,
        expected_revisions: &request.expected_revisions,
        apply_capability: &request.apply_capability,
    })?)
}

#[derive(Debug, Error)]
pub enum TransactionError {
    #[error("typed plan content preparation failed: {0:?}")]
    PlanContent(hiroute_application_api::ErrorCode),
    #[error("invalid transaction arguments")]
    InvalidArguments,
    #[error("cash budget behavior is not enabled in P0")]
    FeatureNotEnabled,
    #[error("authenticated principal lacks this capability")]
    CapabilityDenied,
    #[error("expected revisions changed: {0:?}")]
    RevisionConflict(RevisionMismatch),
    #[error("the reproduced change no longer matches the accepted digest")]
    ChangePreviewStale,
    #[error("the idempotency key is already bound to a different request")]
    IdempotencyKeyReused,
    #[error("the durable Operation was not found")]
    OperationNotFound,
    #[error("protected Secret input is unavailable")]
    ProtectedInputUnavailable,
    #[error("an effect is no longer owned by this Operation")]
    EffectOwnershipLost,
    #[error("an exact Operation effect is missing or remains provisional")]
    EffectMissing,
    #[error("durable writer recovery must complete before accepting writes")]
    RecoveryRequired,
    #[error(transparent)]
    Preparation(#[from] ChangePreparationError),
    #[error(transparent)]
    DomainOperation(#[from] hiroute_domain::OperationValidationError),
    #[error(transparent)]
    CanonicalDigest(#[from] hiroute_domain::CanonicalDigestError),
    #[error(transparent)]
    Port(#[from] PortError),
}

impl TransactionError {
    pub fn error_code(&self) -> ErrorCode {
        match self {
            Self::Port(error) | Self::Preparation(ChangePreparationError::Port(error))
                if error.code == hiroute_domain::PortErrorCode::Conflict
                    && error.context == "qoder.model.budget.shrink" =>
            {
                ErrorCode::QoderModelBudgetConflict
            }
            Self::PlanContent(code) => *code,
            Self::FeatureNotEnabled
            | Self::Preparation(ChangePreparationError::FeatureNotEnabled) => {
                ErrorCode::FeatureNotEnabled
            }
            Self::CapabilityDenied => ErrorCode::CapabilityDenied,
            Self::RevisionConflict(_)
            | Self::Preparation(ChangePreparationError::RuntimeGenerationChanged)
            | Self::Preparation(ChangePreparationError::SecretGenerationChanged) => {
                ErrorCode::RevisionConflict
            }
            Self::ChangePreviewStale
            | Self::Preparation(ChangePreparationError::SecretFingerprintMismatch)
            | Self::Preparation(ChangePreparationError::ExternalEffectChanged) => {
                ErrorCode::ChangePreviewStale
            }
            Self::IdempotencyKeyReused => ErrorCode::IdempotencyKeyReused,
            Self::OperationNotFound => ErrorCode::ResourceNotFound,
            Self::ProtectedInputUnavailable => ErrorCode::ActionRequired,
            Self::EffectOwnershipLost | Self::EffectMissing => ErrorCode::OperationNeedsAttention,
            Self::RecoveryRequired => ErrorCode::DaemonUnavailable,
            Self::Preparation(ChangePreparationError::Port(error)) => {
                if error.code == hiroute_domain::PortErrorCode::NotFound {
                    ErrorCode::ActionRequired
                } else {
                    ErrorCode::DaemonUnavailable
                }
            }
            Self::Port(_) => ErrorCode::DaemonUnavailable,
            _ => ErrorCode::InvalidArguments,
        }
    }

    fn safe_code(&self) -> &'static str {
        match self {
            Self::Port(error) | Self::Preparation(ChangePreparationError::Port(error))
                if error.code == hiroute_domain::PortErrorCode::Conflict
                    && error.context == "qoder.model.budget.shrink" =>
            {
                "QODER_MODEL_BUDGET_CONFLICT"
            }
            Self::ChangePreviewStale
            | Self::Preparation(ChangePreparationError::SecretFingerprintMismatch)
            | Self::Preparation(ChangePreparationError::ExternalEffectChanged) => {
                "CHANGE_PREVIEW_STALE"
            }
            Self::EffectOwnershipLost | Self::EffectMissing => "EFFECT_OWNERSHIP_LOST",
            Self::RecoveryRequired => "RECOVERY_REQUIRED",
            Self::ProtectedInputUnavailable => "PROTECTED_INPUT_UNAVAILABLE",
            Self::Port(error)
                if matches!(
                    error.context,
                    "subscription.source.changed" | "cpa-subscription-source-changed"
                ) =>
            {
                "SUBSCRIPTION_SOURCE_CHANGED"
            }
            Self::Port(error) if error.context == "subscription.authentication" => {
                "SUBSCRIPTION_NEEDS_AUTH"
            }
            Self::Port(error) if error.context == "cpa-subscription-needs-auth" => {
                "SUBSCRIPTION_NEEDS_AUTH"
            }
            Self::Port(error) if error.context == "cpa-subscription-runtime" => {
                "SUBSCRIPTION_RUNTIME_UNAVAILABLE"
            }
            Self::Port(error) if error.context.starts_with("subscription.") => {
                "SUBSCRIPTION_RUNTIME_UNAVAILABLE"
            }
            Self::Port(_) => "ADAPTER_FAILURE",
            _ => "APPLY_STEP_FAILED",
        }
    }
}

#[cfg(test)]
mod tests;
