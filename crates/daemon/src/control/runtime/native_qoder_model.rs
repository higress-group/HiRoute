//! Qoder's additive model file is an ordinary protected settings effect.
use super::LocalControlAdapter;
use hiroute_application::agent_connection::{
    QoderModelFileAction, settings_qoder_model_file_for_operation,
};
use hiroute_domain::{
    AgentAccessGrantRefV1, AgentIngressProtocolV1, AgentModelGrantV2, AgentModelRouteV2,
    ControlRepositoryPort, ExternalEffectIntentV1, GatewayPublicationV1, OperationState,
    OperationStepKind, OperationV1, OwnedEffectV1, PortError, PortErrorCode, PortResult,
    QoderAdditionalModelV1, SecretStorePort, WorkspaceId, is_agent_access_grant_effect,
};

pub(super) fn is_settings_qoder_model(intent: &ExternalEffectIntentV1) -> bool {
    intent.effect_id() == "agent-connection-managed-configuration"
        && intent.desired()["transaction"] == "settings"
        && intent.desired()["subject"]["agent_id"] == "agent_qoder_default"
}

pub(super) fn qoder_model_endpoint(gateway_base_url: &str) -> PortResult<String> {
    let origin = gateway_base_url
        .strip_suffix("/v1")
        .ok_or_else(|| conflict("qoder.settings.endpoint"))?;
    Ok(format!("{origin}{}", hiroute_domain::QODER_MODEL_BASE_PATH))
}

pub(super) fn qoder_models_for_grant(
    publication: &GatewayPublicationV1,
    grant: &AgentModelGrantV2,
) -> PortResult<Vec<QoderAdditionalModelV1>> {
    grant
        .routes
        .iter()
        .map(|(name, route)| {
            let AgentModelRouteV2::Plan {
                plan_id,
                revision,
                semantic_digest,
                ..
            } = route
            else {
                return Err(conflict("qoder.models.route"));
            };
            let plan = publication
                .plans
                .iter()
                .find(|plan| {
                    plan.agent_plan_id() == plan_id && plan.body.agent_plan_revision == *revision
                })
                .ok_or_else(|| conflict("qoder.models.plan"))?
                .clone()
                .into_current()
                .map_err(|_| conflict("qoder.models.plan"))?;
            if &plan.body.materialized_route_digest != semantic_digest {
                return Err(conflict("qoder.models.plan.digest"));
            }
            let budget = hiroute_integrations::qoder_plan_token_budget(&plan.body.materialized)
                .map_err(|_| conflict("qoder.models.budget"))?;
            Ok(QoderAdditionalModelV1 {
                alias: name.clone(),
                context_window_tokens: budget.context_window_tokens,
                max_output_tokens: budget.max_output_tokens,
            })
        })
        .collect()
}

impl LocalControlAdapter {
    pub(super) fn stage_settings_qoder_model(
        &self,
        operation: &OperationV1,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<OwnedEffectV1> {
        if operation.state != OperationState::ApplyingAgentArtifacts {
            return Err(conflict("qoder.settings.phase"));
        }
        let payload = settings_qoder_model_file_for_operation(operation, intent)?;
        let stores = self.stores_lock()?;
        match &payload.change {
            QoderModelFileAction::Configure {
                previous_operation,
                provider_id,
                endpoint,
                models,
            } => {
                let [mutation] = operation.plan.agent_access_grants() else {
                    return Err(conflict("qoder.settings.grant"));
                };
                let scope = mutation
                    .desired_scope()
                    .ok_or_else(|| conflict("qoder.settings.scope"))?;
                if mutation.owner_scope() != operation.workspace_id.as_str()
                    || scope.connection_id() != format!("agent-connection/{}", payload.context_id)
                    || scope.protocol() != AgentIngressProtocolV1::Responses
                {
                    return Err(conflict("qoder.settings.grant.binding"));
                }
                let gateway_base_url = self
                    .managed_agent_runtime
                    .lock()
                    .map_err(|_| conflict("qoder.settings.runtime"))?
                    .as_ref()
                    .map(|runtime| runtime.gateway_base_url.clone())
                    .ok_or_else(|| conflict("qoder.settings.runtime"))?;
                if qoder_model_endpoint(&gateway_base_url)? != *endpoint {
                    return Err(conflict("qoder.settings.endpoint"));
                }
                let active = stores
                    .secrets()
                    .inspect_agent_access_grant(WorkspaceId::DEFAULT, scope.connection_id())?;
                let previous = if let Some(id) = previous_operation {
                    let original = stores
                        .control()
                        .load_operation(id)?
                        .ok_or_else(|| conflict("qoder.settings.previous"))?;
                    let previous_intent = qoder_original_intent(&original, operation, intent)?;
                    let [previous_mutation] = original.plan.agent_access_grants() else {
                        return Err(conflict("qoder.settings.previous.grant"));
                    };
                    let effect = original
                        .step(OperationStepKind::ApplySecrets)
                        .effects
                        .iter()
                        .find(|effect| is_agent_access_grant_effect(effect))
                        .ok_or_else(|| conflict("qoder.settings.previous.effect"))?;
                    let reference =
                        AgentAccessGrantRefV1::from_ensure_effect(effect, previous_mutation)
                            .map_err(|_| conflict("qoder.settings.previous.reference"))?;
                    if active.as_ref() != Some(&reference)
                        || reference.generation() != mutation.expected_generation()
                    {
                        return Err(conflict("qoder.settings.previous.binding"));
                    }
                    Some((original, previous_intent))
                } else {
                    if active.is_some() {
                        return Err(conflict("qoder.settings.previous.missing"));
                    }
                    None
                };
                let material = stores
                    .secrets()
                    .resolve_prepared_agent_access_grant(&operation.operation_id, mutation)?;
                drop(stores);
                let configuration = hiroute_integrations::QoderFileConfiguration {
                    expected_content: &payload.expected_content,
                    provider_id,
                    endpoint,
                    models,
                    local_grant: &material,
                };
                match previous {
                    Some((original, previous_intent)) => {
                        hiroute_integrations::stage_qoder_reconfiguration(
                            &self.artifacts,
                            &operation.operation_id,
                            intent,
                            &original.operation_id,
                            &previous_intent,
                            configuration,
                        )
                    }
                    None => hiroute_integrations::stage_qoder_configuration(
                        &self.artifacts,
                        &operation.operation_id,
                        intent,
                        configuration,
                    ),
                }
            }
            QoderModelFileAction::Restore { original_operation } => {
                let original = stores
                    .control()
                    .load_operation(original_operation)?
                    .ok_or_else(|| conflict("qoder.settings.restore.original"))?;
                let original_intent = qoder_original_intent(&original, operation, intent)?;
                drop(stores);
                hiroute_integrations::stage_qoder_restoration(
                    &self.artifacts,
                    &operation.operation_id,
                    intent,
                    original_operation,
                    &original_intent,
                )
            }
        }
    }
}

fn qoder_original_intent(
    original: &OperationV1,
    operation: &OperationV1,
    intent: &ExternalEffectIntentV1,
) -> PortResult<ExternalEffectIntentV1> {
    if original.workspace_id != operation.workspace_id
        || original.state != OperationState::Succeeded
        || original.operation_id == operation.operation_id
    {
        return Err(conflict("qoder.settings.original.owner"));
    }
    let original_intent = original
        .plan
        .external()
        .iter()
        .find(|candidate| {
            candidate.target() == intent.target() && is_settings_qoder_model(candidate)
        })
        .ok_or_else(|| conflict("qoder.settings.original.intent"))?;
    let before = settings_qoder_model_file_for_operation(original, original_intent)?;
    let after = settings_qoder_model_file_for_operation(operation, intent)?;
    if before.context_id != after.context_id
        || !matches!(before.change, QoderModelFileAction::Configure { .. })
    {
        return Err(conflict("qoder.settings.original.context"));
    }
    Ok(original_intent.clone())
}

fn conflict(context: &'static str) -> PortError {
    PortError::new(PortErrorCode::Conflict, context)
}
