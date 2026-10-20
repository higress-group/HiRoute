//! Persist Agent identity from the same server-owned operation/grant/publication
//! join used to install managed native model settings, before traffic is admitted.
use super::LocalControlAdapter;
use hiroute_domain::{
    AgentAccessGrantMutationKindV1, AgentAccessGrantRefV1, AgentOperationRead,
    GatewayPublicationV1, OperationStepKind, OwnedEffectV1, PortError, PortErrorCode, PortResult,
    WorkspaceId, is_agent_access_grant_effect,
};

fn invalid() -> PortError {
    PortError::new(
        PortErrorCode::InvalidData,
        "observation.agent-grant.provenance",
    )
}

impl LocalControlAdapter {
    pub(super) fn register_operation_agent_grants(
        &self,
        workspace: &WorkspaceId,
        operation: &dyn AgentOperationRead,
        effects: &[OwnedEffectV1],
        publication: &GatewayPublicationV1,
    ) -> PortResult<()> {
        let input = operation.agent_input();
        if input.grants.is_empty() {
            return Ok(());
        }
        let Some(subject) = input.connection_subject().map_err(|_| invalid())? else {
            return Err(invalid());
        };
        for mutation in input.grants {
            if mutation.kind() != AgentAccessGrantMutationKindV1::Ensure {
                continue;
            }
            let reference = effects
                .iter()
                .filter(|effect| is_agent_access_grant_effect(effect))
                .find_map(|effect| AgentAccessGrantRefV1::from_ensure_effect(effect, mutation).ok())
                .ok_or_else(invalid)?;
            if reference.owner_scope() != workspace.as_str() {
                return Err(invalid());
            }
            // Old succeeded operations may own retired grants. Only a grant in
            // this exact verified publication can seed a missing mapping.
            if !publication.grants.iter().any(|grant| {
                grant.grant_id == reference.grant_id()
                    && grant.generation == reference.generation()
                    && grant.bearer_token_sha256 == *reference.material_sha256()
                    && grant.model_grant == *reference.scope().model_grant()
            }) {
                continue;
            }
            self.delegation_observation
                .register_agent_grant(
                    workspace,
                    reference.grant_id(),
                    reference.generation(),
                    reference.connection_id(),
                    subject.agent_id(),
                )
                .map_err(|_| {
                    PortError::new(
                        PortErrorCode::Unavailable,
                        "observation.agent-grant.persist",
                    )
                })?;
        }
        Ok(())
    }

    pub(super) fn register_restored_agent_grants(
        &self,
        workspace: &WorkspaceId,
        publication: &GatewayPublicationV1,
    ) -> PortResult<()> {
        let operations = self
            .stores_lock()?
            .control()
            .succeeded_agent_operations_for_kind(workspace, "ApplyAgentConnectionChange")?;
        for operation in operations {
            self.register_operation_agent_grants(
                workspace,
                &operation,
                &operation.step(OperationStepKind::ApplySecrets).effects,
                publication,
            )?;
        }
        Ok(())
    }
}
