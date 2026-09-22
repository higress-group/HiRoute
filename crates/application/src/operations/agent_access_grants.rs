use hiroute_domain::{EffectReconciliation, OperationStepKind, OperationV1, SecretStorePort};

use super::{TransactionCoordinator, TransactionError, require_applied, require_missing};

impl<'a, C, S, R, E, I> TransactionCoordinator<'a, C, S, R, E, I>
where
    C: hiroute_domain::ControlRepositoryPort
        + hiroute_domain::ComputeSourceControlPort
        + hiroute_domain::CredentialPoolControlPort
        + crate::ConnectionOptionAuthorizationPort,
    S: SecretStorePort,
    R: hiroute_domain::RuntimeStatePort,
    E: hiroute_domain::ExternalEffectPort,
    I: crate::ProtectedInputPort,
{
    pub(super) fn stage_agent_access_grants(
        &self,
        operation: &mut OperationV1,
    ) -> Result<(), TransactionError> {
        for mutation in operation.plan.agent_access_grants().to_vec() {
            let effect = match self
                .secrets
                .observe_agent_access_grant(&operation.operation_id, &mutation)?
            {
                EffectReconciliation::Staged(effect) | EffectReconciliation::Applied(effect) => {
                    effect
                }
                EffectReconciliation::Missing => {
                    let input = mutation
                        .material_action()
                        .input()
                        .map(|(slot, fingerprint)| {
                            let input =
                                self.protected_inputs.read_secret(slot).map_err(|error| {
                                    if error.code == hiroute_domain::PortErrorCode::NotFound {
                                        TransactionError::ProtectedInputUnavailable
                                    } else {
                                        TransactionError::Port(error)
                                    }
                                })?;
                            if self.secrets.fingerprint(&input)? != *fingerprint {
                                return Err(TransactionError::ChangePreviewStale);
                            }
                            Ok(input)
                        })
                        .transpose()?;
                    self.secrets.apply_agent_access_grant(
                        &operation.operation_id,
                        &mutation,
                        input.as_ref(),
                    )?
                }
                EffectReconciliation::OwnershipLost(effect) => {
                    self.record_effect(operation, OperationStepKind::ApplySecrets, effect)?;
                    return Err(TransactionError::EffectOwnershipLost);
                }
            };
            self.record_effect(operation, OperationStepKind::ApplySecrets, effect)?;
        }
        Ok(())
    }

    pub(super) fn activate_agent_access_grants(
        &self,
        operation: &mut OperationV1,
    ) -> Result<(), TransactionError> {
        for mutation in operation.plan.agent_access_grants().to_vec() {
            let effect = match self
                .secrets
                .observe_agent_access_grant(&operation.operation_id, &mutation)?
            {
                EffectReconciliation::Staged(effect) => {
                    self.secrets.activate_agent_access_grant(&effect)?
                }
                EffectReconciliation::Applied(effect) => effect,
                EffectReconciliation::Missing => return Err(TransactionError::EffectMissing),
                EffectReconciliation::OwnershipLost(_) => {
                    return Err(TransactionError::EffectOwnershipLost);
                }
            };
            self.record_effect(operation, OperationStepKind::ApplySecrets, effect)?;
        }
        Ok(())
    }

    pub(super) fn require_agent_access_grants_applied(
        &self,
        operation: &OperationV1,
    ) -> Result<(), TransactionError> {
        for mutation in operation.plan.agent_access_grants() {
            require_applied(
                self.secrets
                    .observe_agent_access_grant(&operation.operation_id, mutation)?,
            )?;
        }
        Ok(())
    }

    pub(super) fn require_agent_access_grants_missing(
        &self,
        operation: &OperationV1,
    ) -> Result<(), TransactionError> {
        for mutation in operation.plan.agent_access_grants() {
            require_missing(
                self.secrets
                    .observe_agent_access_grant(&operation.operation_id, mutation)?,
            )?;
        }
        Ok(())
    }

    pub(super) fn reconcile_agent_access_grants_for_rollback(
        &self,
        operation: &mut OperationV1,
    ) -> Result<(), TransactionError> {
        let mut ownership_lost = false;
        for mutation in operation.plan.agent_access_grants().to_vec() {
            match self
                .secrets
                .observe_agent_access_grant(&operation.operation_id, &mutation)?
            {
                EffectReconciliation::Missing => {}
                EffectReconciliation::Staged(effect) | EffectReconciliation::Applied(effect) => {
                    self.record_effect(operation, OperationStepKind::ApplySecrets, effect)?;
                }
                EffectReconciliation::OwnershipLost(effect) => {
                    self.record_effect(operation, OperationStepKind::ApplySecrets, effect)?;
                    ownership_lost = true;
                }
            }
        }
        if ownership_lost {
            Err(TransactionError::EffectOwnershipLost)
        } else {
            Ok(())
        }
    }
}
