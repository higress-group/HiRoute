use super::*;
use hiroute_domain::{
    ComputeManagementMutation, DecisionServiceOperationV2, VerifiedSecretSubjectV1,
};

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
    pub(super) fn reconcile_compute_compensation(
        &self,
        operation: &OperationV1,
    ) -> Result<(), TransactionError> {
        let mut expected_sources = Vec::new();
        if let Some(value) = operation.plan.control().get("compute_management_mutation") {
            let mutation: ComputeManagementMutation = serde_json::from_value(value.clone())
                .map_err(|_| TransactionError::EffectOwnershipLost)?;
            if let Some(expected) = mutation.expected() {
                expected_sources.push(expected.clone());
            }
        } else if let Some(value) = operation
            .plan
            .control()
            .get("decision_service_operation_v2")
        {
            let decision: DecisionServiceOperationV2 = serde_json::from_value(value.clone())
                .map_err(|_| TransactionError::EffectOwnershipLost)?;
            expected_sources.extend(decision.sources.into_iter().map(|source| source.before));
        }
        let mut references = Vec::new();
        for secret in operation.plan.secrets() {
            let Some(key) = expected_sources
                .iter()
                .flat_map(|source| &source.credentials)
                .find(|key| key.key_id == secret.credential().credential_id())
            else {
                continue;
            };
            let reference = self
                .secrets
                .compensated_secret_reference(&operation.operation_id, secret)?
                .ok_or(TransactionError::EffectMissing)?;
            let subject = VerifiedSecretSubjectV1::from_authenticated_transport(
                reference.subject(),
                reference.owner_scope(),
            )
            .map_err(|_| TransactionError::EffectOwnershipLost)?;
            let destination = reference
                .allowed_destinations()
                .iter()
                .next()
                .ok_or(TransactionError::EffectMissing)?;
            let restored = self.secrets.resolve_secret(
                &subject,
                &reference,
                reference.purpose(),
                destination,
                reference.generation(),
            )?;
            if self.secrets.fingerprint(&restored)? != key.fingerprint {
                return Err(TransactionError::EffectOwnershipLost);
            }
            if reference != key.credential {
                references.push(reference);
            }
        }
        if !references.is_empty() {
            self.control
                .reconcile_compute_compensation(operation, &references)?;
        }
        Ok(())
    }
}
