//! Native Claude settings effects in the production Operation composition.
use super::LocalControlAdapter;
use hiroute_application::agent_connection::{
    ClaudeModelFileAction, settings_claude_model_file_for_operation,
};
use hiroute_domain::{
    AgentAccessGrantRefV1, AgentIngressProtocolV1, ControlRepositoryPort, ExternalEffectIntentV1,
    OperationState, OperationStepKind, OwnedEffectV1, PortError, PortErrorCode, PortResult,
    SecretStorePort, WorkspaceId, is_agent_access_grant_effect,
};
use hiroute_integrations::{
    stage_claude_configuration, stage_claude_reconfiguration, stage_claude_restoration,
};

pub(super) fn is_settings_claude_model(intent: &ExternalEffectIntentV1) -> bool {
    intent.effect_id() == "agent-connection-managed-configuration"
        && intent.desired()["transaction"] == "settings"
        && intent.desired()["subject"]["agent_id"] == "agent_claude_default"
}

impl LocalControlAdapter {
    pub(super) fn stage_settings_claude_model(
        &self,
        operation: &hiroute_domain::OperationV1,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<OwnedEffectV1> {
        let stores = self.stores_lock()?;
        let operation_id = &operation.operation_id;
        if operation.state != OperationState::ApplyingAgentArtifacts {
            return Err(conflict("claude.settings.phase"));
        }
        let payload = settings_claude_model_file_for_operation(operation, intent)?;
        match &payload.change {
            ClaudeModelFileAction::Configure {
                previous_operation,
                change,
                snapshot,
                gateway_base_url,
                trusted_hiroute_executable,
            } => {
                let mutation = operation
                    .plan
                    .agent_access_grants()
                    .first()
                    .filter(|_| operation.plan.agent_access_grants().len() == 1)
                    .ok_or_else(|| conflict("claude.settings.grant"))?;
                let scope = mutation
                    .desired_scope()
                    .ok_or_else(|| conflict("claude.settings.scope"))?;
                if mutation.owner_scope() != operation.workspace_id.as_str()
                    || scope.connection_id() != format!("agent-connection/{}", payload.context_id)
                    || scope.protocol() != AgentIngressProtocolV1::Messages
                {
                    return Err(conflict("claude.settings.grant.binding"));
                }
                let runtime = self
                    .managed_agent_runtime
                    .lock()
                    .map_err(|_| conflict("claude.settings.runtime.lock"))?;
                if runtime.as_ref().is_none_or(|runtime| {
                    runtime.gateway_base_url != *gateway_base_url
                        || runtime.trusted_hiroute_executable != *trusted_hiroute_executable
                }) {
                    return Err(conflict("claude.settings.runtime.changed"));
                }
                drop(runtime);
                let active = stores
                    .secrets()
                    .inspect_agent_access_grant(WorkspaceId::DEFAULT, scope.connection_id())?;
                if let Some(previous_operation) = previous_operation {
                    let previous = stores
                        .control()
                        .load_operation(previous_operation)?
                        .ok_or_else(|| conflict("claude.settings.previous.operation"))?;
                    if previous.workspace_id != operation.workspace_id
                        || previous.state != OperationState::Succeeded
                    {
                        return Err(conflict("claude.settings.previous.owner"));
                    }
                    let previous_intent = previous
                        .plan
                        .external()
                        .iter()
                        .find(|candidate| {
                            candidate.target() == intent.target()
                                && is_settings_claude_model(candidate)
                        })
                        .ok_or_else(|| conflict("claude.settings.previous.effect"))?
                        .clone();
                    let previous_payload =
                        settings_claude_model_file_for_operation(&previous, &previous_intent)?;
                    if !matches!(
                        previous_payload.change,
                        ClaudeModelFileAction::Configure { .. }
                    ) {
                        return Err(conflict("claude.settings.previous.action"));
                    }
                    let [previous_mutation] = previous.plan.agent_access_grants() else {
                        return Err(conflict("claude.settings.previous.grant"));
                    };
                    let previous_effect = previous
                        .step(OperationStepKind::ApplySecrets)
                        .effects
                        .iter()
                        .find(|effect| is_agent_access_grant_effect(effect))
                        .ok_or_else(|| conflict("claude.settings.previous.grant.effect"))?;
                    let previous_reference = AgentAccessGrantRefV1::from_ensure_effect(
                        previous_effect,
                        previous_mutation,
                    )
                    .map_err(|_| conflict("claude.settings.previous.grant.reference"))?;
                    if active.as_ref() != Some(&previous_reference)
                        || previous_reference.generation() != mutation.expected_generation()
                    {
                        return Err(conflict("claude.settings.previous.binding"));
                    }
                } else if active.is_some() {
                    return Err(conflict("claude.settings.previous.missing"));
                }
                drop(stores);
                let _ = snapshot;
                if let Some(previous_operation) = previous_operation {
                    let stores = self.stores_lock()?;
                    let previous = stores
                        .control()
                        .load_operation(previous_operation)?
                        .ok_or_else(|| conflict("claude.settings.previous.operation"))?;
                    let previous_intent = previous
                        .plan
                        .external()
                        .iter()
                        .find(|candidate| {
                            candidate.target() == intent.target()
                                && is_settings_claude_model(candidate)
                        })
                        .ok_or_else(|| conflict("claude.settings.previous.effect"))?;
                    stage_claude_reconfiguration(
                        &self.artifacts,
                        operation_id,
                        intent,
                        previous_operation,
                        previous_intent,
                        &payload.expected_content,
                        change,
                    )
                } else {
                    stage_claude_configuration(
                        &self.artifacts,
                        operation_id,
                        intent,
                        &payload.expected_content,
                        change,
                    )
                }
            }
            ClaudeModelFileAction::Restore { original_operation } => {
                let original = stores
                    .control()
                    .load_operation(original_operation)?
                    .ok_or_else(|| conflict("claude.settings.restore.original"))?;
                if original.workspace_id != operation.workspace_id
                    || original.state != OperationState::Succeeded
                {
                    return Err(conflict("claude.settings.restore.owner"));
                }
                let original_intent = original
                    .plan
                    .external()
                    .iter()
                    .find(|original| {
                        original.target() == intent.target() && is_settings_claude_model(original)
                    })
                    .ok_or_else(|| conflict("claude.settings.restore.effect"))?;
                let before = settings_claude_model_file_for_operation(&original, original_intent)?;
                if before.context_id != payload.context_id
                    || !matches!(before.change, ClaudeModelFileAction::Configure { .. })
                {
                    return Err(conflict("claude.settings.restore.context"));
                }
                stage_claude_restoration(
                    &self.artifacts,
                    operation_id,
                    intent,
                    original_operation,
                    original_intent,
                )
            }
        }
    }
}

fn conflict(context: &'static str) -> PortError {
    PortError::new(PortErrorCode::Conflict, context)
}
