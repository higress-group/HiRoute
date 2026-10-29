//! Persist Agent identity from the same server-owned operation/grant/publication
//! join used to install managed native model settings, before traffic is admitted.
use super::LocalControlAdapter;
use hiroute_domain::{
    AgentAccessGrantMutationKindV1, AgentAccessGrantRefV1, AgentOperationRead, CanonicalDigest,
    ControlRepositoryPort, GatewayPublicationV1, OperationStepKind, OwnedEffectV1, PortError,
    PortErrorCode, PortResult, WorkspaceId, is_agent_access_grant_effect,
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
        publication_digest: &CanonicalDigest,
    ) -> PortResult<()> {
        let (operations, mut rejected) = {
            let stores = self.stores_lock()?;
            let owners = stores
                .secrets()
                .active_agent_grant_owner_operations(workspace)?;
            stores
                .control()
                .agent_attribution_for_owners(workspace, &owners)?
        };
        for operation in operations {
            let input = operation.agent_input();
            let effects = operation.grant_effects();
            for mutation in input.grants {
                if mutation.kind() != AgentAccessGrantMutationKindV1::Ensure {
                    continue;
                }
                let reference = effects.iter().find_map(|effect| {
                    AgentAccessGrantRefV1::from_ensure_effect(effect, mutation).ok()
                });
                let Some(reference) = reference else {
                    rejected = rejected.saturating_add(1);
                    continue;
                };
                if !publication.grants.iter().any(|grant| {
                    grant.grant_id == reference.grant_id()
                        && grant.generation == reference.generation()
                        && grant.bearer_token_sha256 == *reference.material_sha256()
                        && grant.model_grant == *reference.scope().model_grant()
                }) {
                    continue;
                }
                // Only exact current grant evidence needs a subject. Retired or unreadable
                // history cannot block unrelated current traffic or fabricate attribution.
                let subject = input.connection_subject().ok().flatten();
                let Some(subject) =
                    subject.filter(|_| reference.owner_scope() == workspace.as_str())
                else {
                    rejected = rejected.saturating_add(1);
                    continue;
                };
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
        }
        // A committed service can keep serving while its independently edited native file
        // remains a parked tail. Its authenticated receipt, not Succeeded alone, proves the
        // current publication. Reuse the existing recovery proof and exact journal guard;
        // this neither retries the client file nor exports an unfinished connection grant.
        let pending = self.stores_lock()?.control().recoverable_operations()?;
        for operation in pending {
            if operation.workspace_id != *workspace {
                continue;
            }
            if !hiroute_application::settings_service_completion_is_current(
                &operation,
                publication.publication_revision.get(),
                publication_digest,
            ) {
                if !operation.agent_input().grants.is_empty() {
                    rejected = rejected.saturating_add(1);
                }
                continue;
            }
            self.require_current_operation(&operation)?;
            self.register_operation_agent_grants(
                workspace,
                &operation,
                &operation.step(OperationStepKind::ApplySecrets).effects,
                publication,
            )?;
        }
        if rejected > 0 {
            // One closed-vocabulary warning per reconciliation, irrespective of history
            // size. No operation contents, identities or error strings enter diagnostics.
            use hiroute_diagnostics::event::{
                DiagnosticEvent, ObservationChannel, ProjectionReason, ProjectionRejected,
            };
            if let Ok(diagnostics) = self.publication_diagnostics.lock() {
                diagnostics.emit(DiagnosticEvent::ProjectionRejected(ProjectionRejected {
                    channel: ObservationChannel::Control,
                    reason: ProjectionReason::Invalid,
                }));
            }
        }
        // Publication activation and its service receipt are separate transactions.
        // Missing attribution proof must leave recovery open: the journal can finish the
        // operation, and the subsequent restore registers its authenticated ownership.
        Ok(())
    }
}
