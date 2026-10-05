//! Shared additional-provider settings effects; native cross-file dependencies remain explicit.
use super::LocalControlAdapter;
use hiroute_application::agent_connection::{
    AdditionalModelFileAction, additional_model_kind, settings_additional_model_file_for_operation,
};
use hiroute_domain::{
    AdditionalAgentModelV1, AgentAccessGrantRefV1, AgentKindV1, AgentModelGrantV2,
    AgentModelRouteV2, ControlRepositoryPort, ExternalEffectIntentV1, GatewayPublicationV1,
    OperationState, OperationStepKind, OperationV1, OwnedEffectV1, PortError, PortErrorCode,
    PortResult, SecretStorePort, WorkspaceId, is_agent_access_grant_effect,
};

pub(super) fn is_settings_additional_model(intent: &ExternalEffectIntentV1) -> bool {
    intent.effect_id() == "agent-connection-managed-configuration"
        && intent.desired()["transaction"] == "settings"
        && matches!(
            intent.desired()["subject"]["agent_id"].as_str(),
            Some("agent_qoder_default" | "agent_pi_default")
        )
}

pub(super) fn additional_model_endpoint(
    kind: AgentKindV1,
    gateway_base_url: &str,
) -> PortResult<String> {
    if kind == AgentKindV1::Pi {
        return Ok(gateway_base_url.to_owned());
    }
    let origin = gateway_base_url
        .strip_suffix("/v1")
        .ok_or_else(|| conflict("qoder.settings.endpoint"))?;
    Ok(format!("{origin}{}", hiroute_domain::QODER_MODEL_BASE_PATH))
}

pub(super) fn additional_models_for_grant(
    kind: AgentKindV1,
    publication: &GatewayPublicationV1,
    grant: &AgentModelGrantV2,
) -> PortResult<Vec<AdditionalAgentModelV1>> {
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
            let budget = (if kind == AgentKindV1::Pi {
                hiroute_integrations::pi_plan_token_budget(&plan.body.materialized)
            } else {
                hiroute_integrations::qoder_plan_token_budget(&plan.body.materialized)
            })
            .map_err(|_| conflict("qoder.models.budget"))?;
            Ok(AdditionalAgentModelV1 {
                alias: name.clone(),
                protocol: grant.protocol_for(name),
                context_window_tokens: budget.context_window_tokens,
                max_output_tokens: budget.max_output_tokens,
            })
        })
        .collect()
}

impl LocalControlAdapter {
    /// Recheck independent native files at stage and final activation, including recovery.
    pub(super) fn validate_additional_model_dependencies(
        &self,
        operation: &OperationV1,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<()> {
        let payload = settings_additional_model_file_for_operation(operation, intent)?;
        if let Some(expected) = &payload.pi_settings_content {
            let settings = self
                .scanner
                .pi_default_model()
                .map_err(|_| conflict("pi.settings.default"))?;
            let models = match &payload.change {
                AdditionalModelFileAction::Configure { models, .. } => models.as_slice(),
                _ => &[],
            };
            if &settings.content_digest != expected
                || settings.removes_default(
                    &hiroute_application::agent_connection::additional_model_provider_id(
                        &payload.context_id,
                    ),
                    models,
                )
            {
                return Err(conflict("pi.settings.default.changed"));
            }
        }
        Ok(())
    }

    pub(super) fn stage_settings_additional_model(
        &self,
        operation: &OperationV1,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<OwnedEffectV1> {
        if operation.state != OperationState::ApplyingAgentArtifacts {
            return Err(conflict("qoder.settings.phase"));
        }
        let payload = settings_additional_model_file_for_operation(operation, intent)?;
        let kind = additional_model_kind(intent)?;
        self.validate_additional_model_dependencies(operation, intent)?;
        let stores = self.stores_lock()?;
        match &payload.change {
            AdditionalModelFileAction::Configure {
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
                if additional_model_endpoint(kind, &gateway_base_url)? != *endpoint {
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
                    let previous_intent = additional_original_intent(&original, operation, intent)?;
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
                let configuration = hiroute_integrations::AdditionalFileConfiguration {
                    expected_content: &payload.expected_content,
                    provider_id,
                    endpoint,
                    models,
                    local_grant: &material,
                };
                match previous {
                    Some((original, previous_intent)) => {
                        hiroute_integrations::stage_additional_reconfiguration(
                            &self.artifacts,
                            &operation.operation_id,
                            intent,
                            &original.operation_id,
                            &previous_intent,
                            configuration,
                        )
                    }
                    None => hiroute_integrations::stage_additional_configuration(
                        &self.artifacts,
                        &operation.operation_id,
                        intent,
                        configuration,
                    ),
                }
            }
            AdditionalModelFileAction::Restore { original_operation } => {
                let original = stores
                    .control()
                    .load_operation(original_operation)?
                    .ok_or_else(|| conflict("qoder.settings.restore.original"))?;
                let original_intent = additional_original_intent(&original, operation, intent)?;
                drop(stores);
                hiroute_integrations::stage_additional_restoration(
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

fn additional_original_intent(
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
            candidate.target() == intent.target() && is_settings_additional_model(candidate)
        })
        .ok_or_else(|| conflict("qoder.settings.original.intent"))?;
    let before = settings_additional_model_file_for_operation(original, original_intent)?;
    let after = settings_additional_model_file_for_operation(operation, intent)?;
    if before.context_id != after.context_id
        || !matches!(before.change, AdditionalModelFileAction::Configure { .. })
    {
        return Err(conflict("qoder.settings.original.context"));
    }
    Ok(original_intent.clone())
}

fn conflict(context: &'static str) -> PortError {
    PortError::new(PortErrorCode::Conflict, context)
}
