//! Hermes native effects use original accepted scopes and the existing journal.
use super::LocalControlAdapter;
use hiroute_application::agent_connection::{
    HermesModelFileAction, NativePluginFileAction, is_settings_hermes_model,
    is_settings_native_plugin, settings_hermes_model_file_for_operation,
    settings_native_plugin_file_for_operation,
};
use hiroute_domain::{
    AgentAccessGrantRefV1, AgentSettingsSpecV2, ControlRepositoryPort, ExternalEffectIntentV1,
    OperationState, OperationStepKind, OperationV1, OwnedEffectV1, PortError, PortErrorCode,
    PortResult, SecretStorePort, WorkspaceId, is_agent_access_grant_effect,
};

impl LocalControlAdapter {
    fn register_hermes_operation_target(&self, operation: &OperationV1) -> PortResult<()> {
        let spec: AgentSettingsSpecV2 =
            serde_json::from_value(operation.plan.spec().desired_state.clone())
                .map_err(|_| invalid("hermes.operation.spec"))?;
        self.hermes_target_for_spec(&spec)
            .map_err(|_| conflict("hermes.operation.target"))?;
        Ok(())
    }

    pub(super) fn validate_hermes_model_dependencies(
        &self,
        operation: &OperationV1,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<()> {
        self.register_hermes_operation_target(operation)?;
        let payload = settings_hermes_model_file_for_operation(operation, intent)?;
        if matches!(payload.change, HermesModelFileAction::Configure { .. })
            && !hiroute_integrations::hermes_support_facts().middleware
        {
            return Err(conflict("hermes.native.middleware.unavailable"));
        }
        Ok(())
    }

    pub(super) fn stage_settings_hermes_model(
        &self,
        operation: &OperationV1,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<OwnedEffectV1> {
        if operation.state != OperationState::ApplyingAgentArtifacts {
            return Err(conflict("hermes.operation.phase"));
        }
        self.validate_hermes_model_dependencies(operation, intent)?;
        let payload = settings_hermes_model_file_for_operation(operation, intent)?;
        let stores = self.stores_lock()?;
        match &payload.change {
            HermesModelFileAction::Configure {
                previous_operation,
                provider_id,
                endpoint,
                model,
            } => {
                let [mutation] = operation.plan.agent_access_grants() else {
                    return Err(invalid("hermes.grant.count"));
                };
                let scope = mutation
                    .desired_scope()
                    .ok_or_else(|| invalid("hermes.grant.scope"))?;
                if mutation.owner_scope() != operation.workspace_id.as_str()
                    || scope.connection_id() != format!("agent-connection/{}", payload.context_id)
                {
                    return Err(invalid("hermes.grant.binding"));
                }
                let base = self
                    .managed_agent_runtime
                    .lock()
                    .map_err(|_| conflict("hermes.gateway.runtime"))?
                    .as_ref()
                    .map(|r| r.gateway_base_url.clone())
                    .ok_or_else(|| conflict("hermes.gateway.runtime"))?;
                if &base != endpoint {
                    return Err(conflict("hermes.gateway.endpoint"));
                }
                let active = stores
                    .secrets()
                    .inspect_agent_access_grant(WorkspaceId::DEFAULT, scope.connection_id())?;
                let previous = if let Some(id) = previous_operation {
                    let original = stores
                        .control()
                        .load_operation(id)?
                        .ok_or_else(|| invalid("hermes.previous.operation"))?;
                    let previous_intent = original_intent(&original, operation, intent, false)?;
                    let [previous_mutation] = original.plan.agent_access_grants() else {
                        return Err(invalid("hermes.previous.grant"));
                    };
                    let effect = original
                        .step(OperationStepKind::ApplySecrets)
                        .effects
                        .iter()
                        .find(|effect| is_agent_access_grant_effect(effect))
                        .ok_or_else(|| invalid("hermes.previous.effect"))?;
                    let reference =
                        AgentAccessGrantRefV1::from_ensure_effect(effect, previous_mutation)
                            .map_err(|_| invalid("hermes.previous.reference"))?;
                    if active.as_ref() != Some(&reference)
                        || reference.generation() != mutation.expected_generation()
                    {
                        return Err(conflict("hermes.previous.binding"));
                    }
                    Some((original, previous_intent))
                } else {
                    if active.is_some() {
                        return Err(conflict("hermes.previous.missing"));
                    }
                    None
                };
                let material = stores
                    .secrets()
                    .resolve_prepared_agent_access_grant(&operation.operation_id, mutation)?;
                drop(stores);
                hiroute_integrations::stage_hermes_configuration(
                    &self.artifacts,
                    &operation.operation_id,
                    intent,
                    hiroute_integrations::HermesFileConfiguration {
                        expected_content: &payload.expected_content,
                        provider_id,
                        endpoint,
                        model,
                        local_grant: &material,
                    },
                    previous
                        .as_ref()
                        .map(|(op, original)| (&op.operation_id, original)),
                )
            }
            HermesModelFileAction::Restore { original_operation } => {
                let original = stores
                    .control()
                    .load_operation(original_operation)?
                    .ok_or_else(|| invalid("hermes.restore.original"))?;
                let original_intent = original_intent(&original, operation, intent, false)?;
                drop(stores);
                hiroute_integrations::stage_hermes_restoration(
                    &self.artifacts,
                    &operation.operation_id,
                    intent,
                    original_operation,
                    &original_intent,
                )
            }
        }
    }

    pub(super) fn stage_settings_hermes_plugin(
        &self,
        operation: &OperationV1,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<OwnedEffectV1> {
        if operation.state != OperationState::ApplyingAgentArtifacts {
            return Err(conflict("hermes.plugin.phase"));
        }
        if intent.effect_id() != "agent-connection-native-routing-adapter" {
            self.register_hermes_operation_target(operation)?;
        }
        let payload = settings_native_plugin_file_for_operation(operation, intent)?;
        let original_id = match &payload.change {
            NativePluginFileAction::Configure {
                previous_operation, ..
            } => previous_operation.as_ref(),
            NativePluginFileAction::Restore { original_operation } => Some(original_operation),
        };
        let previous = if let Some(id) = original_id {
            let original = self
                .stores_lock()?
                .control()
                .load_operation(id)?
                .ok_or_else(|| invalid("hermes.plugin.previous"))?;
            if intent.effect_id() == "agent-connection-native-routing-adapter"
                && matches!(&payload.change, NativePluginFileAction::Configure { .. })
                && !original
                    .plan
                    .external()
                    .iter()
                    .any(|i| i.effect_id() == "agent-connection-native-routing-adapter")
            {
                if original.workspace_id != operation.workspace_id
                    || original.state != OperationState::Succeeded
                    || original.plan.spec().resource_id != operation.plan.spec().resource_id
                {
                    return Err(invalid("native.plugin.previous.owner"));
                }
                None
            } else {
                let old_intent = original_intent(&original, operation, intent, true)?;
                Some((original, old_intent))
            }
        } else {
            None
        };
        match &payload.change {
            NativePluginFileAction::Configure { content, .. } => {
                hiroute_integrations::stage_native_plugin_configuration(
                    &self.artifacts,
                    &operation.operation_id,
                    intent,
                    &payload.expected_content,
                    content.as_bytes(),
                    previous.as_ref().map(|(op, i)| (&op.operation_id, i)),
                )
            }
            NativePluginFileAction::Restore { original_operation } => {
                let (_, original) = previous
                    .as_ref()
                    .ok_or_else(|| invalid("hermes.plugin.restore.original"))?;
                hiroute_integrations::stage_native_plugin_restoration(
                    &self.artifacts,
                    &operation.operation_id,
                    intent,
                    &payload.expected_content,
                    original_operation,
                    original,
                )
            }
        }
    }

    pub(super) fn hermes_operation_files_applied(
        &self,
        operation: &(impl hiroute_domain::AgentOperationRead + ?Sized),
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<bool> {
        if !hiroute_integrations::hermes_native_configuration_is_applied(
            &self.artifacts,
            operation.operation_id(),
            intent,
        )? {
            return Ok(false);
        }
        self.hermes_operation_plugin_files_applied(operation)
    }

    pub(super) fn hermes_operation_plugin_files_applied(
        &self,
        operation: &(impl hiroute_domain::AgentOperationRead + ?Sized),
    ) -> PortResult<bool> {
        let plugins: Vec<_> = operation
            .agent_input()
            .external
            .iter()
            .filter(|i| is_settings_native_plugin(i))
            .collect();
        if plugins.len() != 2 {
            return Err(invalid("hermes.plugin.files.count"));
        }
        for plugin in plugins {
            settings_native_plugin_file_for_operation(operation, plugin)?;
            if !hiroute_integrations::native_plugin_is_applied(
                &self.artifacts,
                operation.operation_id(),
                plugin,
            )? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

fn original_intent(
    original: &OperationV1,
    operation: &OperationV1,
    intent: &ExternalEffectIntentV1,
    plugin: bool,
) -> PortResult<ExternalEffectIntentV1> {
    if original.workspace_id != operation.workspace_id
        || original.state != OperationState::Succeeded
        || original.operation_id == operation.operation_id
    {
        return Err(invalid("hermes.original.owner"));
    }
    let original_intent = original
        .plan
        .external()
        .iter()
        .find(|i| {
            i.target() == intent.target()
                && if plugin {
                    is_settings_native_plugin(i)
                } else {
                    is_settings_hermes_model(i)
                }
        })
        .ok_or_else(|| invalid("hermes.original.intent"))?;
    if plugin {
        let before = settings_native_plugin_file_for_operation(original, original_intent)?;
        let after = settings_native_plugin_file_for_operation(operation, intent)?;
        if before.context_id != after.context_id
            || !matches!(before.change, NativePluginFileAction::Configure { .. })
        {
            return Err(invalid("hermes.original.context"));
        }
    } else {
        let before = settings_hermes_model_file_for_operation(original, original_intent)?;
        let after = settings_hermes_model_file_for_operation(operation, intent)?;
        if before.context_id != after.context_id
            || !matches!(before.change, HermesModelFileAction::Configure { .. })
        {
            return Err(invalid("hermes.original.context"));
        }
    }
    Ok(original_intent.clone())
}

fn invalid(context: &'static str) -> PortError {
    PortError::new(PortErrorCode::InvalidData, context)
}
fn conflict(context: &'static str) -> PortError {
    PortError::new(PortErrorCode::Conflict, context)
}
