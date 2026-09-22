//! Native Codex settings effects in the production Operation composition.
use super::LocalControlAdapter;
use hiroute_application::agent_connection::{
    CodexCatalogProducerKindV1, CodexModelFileAction, decode_settings_codex_catalog,
    settings_codex_model_file_for_operation,
};
use hiroute_domain::{
    AgentAccessGrantRefV1, AgentIngressProtocolV1, AgentModelDefaultSelectionV2, AgentModelRouteV2,
    AgentModelSelectionV2, CanonicalDigest, CodexNativeModelModeV2, ControlRepositoryPort,
    EffectReconciliation, ExternalEffectIntentV1, GatewayPublicationV1, NativeAgentArtifactPort,
    OperationId, OperationState, OperationStepKind, OwnedEffectV1, PortError, PortErrorCode,
    PortResult, SecretStorePort, WorkspaceId, is_agent_access_grant_effect,
};
use hiroute_integrations::{
    CodexCatalogBaseline, CodexCatalogError, CodexDefaultPolicy, CodexFileConfiguration,
    CodexSelectionTarget, codex_catalog_baseline as decode_catalog_baseline, restage_codex_catalog,
    sample_codex_catalog_plan, sample_codex_hiroute_only_catalog_plan, stage_codex_catalog,
    stage_codex_configuration, stage_codex_reconfiguration, stage_codex_restoration,
};

pub(super) fn is_settings_codex_model(intent: &ExternalEffectIntentV1) -> bool {
    intent.effect_id() == "agent-connection-managed-configuration"
        && intent.desired()["transaction"] == "settings"
        && intent.desired()["subject"]["agent_id"] == "agent_codex_default"
}

pub(super) fn is_settings_codex_catalog(intent: &ExternalEffectIntentV1) -> bool {
    intent.effect_id() == "agent-connection-model-catalog"
        && intent.desired()["transaction"] == "settings"
        && intent.desired()["subject"]["agent_id"] == "agent_codex_default"
}

/// The catalog baseline of the active managed edit: the user's pre-management pointer, so a
/// reconfiguration never appends plans onto a previous HiRoute catalog artifact.
pub(super) fn codex_catalog_baseline(
    control: &dyn ControlRepositoryPort,
    artifacts: &dyn NativeAgentArtifactPort,
    previous: Option<&OperationId>,
    model_target: &str,
) -> PortResult<CodexCatalogBaseline> {
    let Some(previous) = previous else {
        return Ok(CodexCatalogBaseline::Scope);
    };
    let operation = control
        .load_operation(previous)?
        .ok_or_else(|| conflict("codex.catalog.previous.operation"))?;
    let intent = operation
        .plan
        .external()
        .iter()
        .find(|candidate| is_settings_codex_model(candidate) && candidate.target() == model_target)
        .ok_or_else(|| conflict("codex.catalog.previous.intent"))?;
    let record = artifacts
        .load_native_restore(previous, intent)?
        .ok_or_else(|| conflict("codex.catalog.previous.record"))?;
    decode_catalog_baseline(&record).map_err(|_| conflict("codex.catalog.previous.baseline"))
}

/// Deterministic derivation of the merged catalog from a plan-carrying selection: the same
/// publication, plan order and default policy seal, stage and recovery all share.
pub(super) fn codex_catalog_plan_for(
    user_config: &std::path::Path,
    settings: &AgentModelSelectionV2,
    publication: &GatewayPublicationV1,
    baseline: &CodexCatalogBaseline,
) -> Result<hiroute_integrations::CodexCatalogPlan, CodexCatalogError> {
    let AgentModelSelectionV2::CodexDefault {
        default_selection,
        native_model_mode,
        fixed_models,
        ..
    } = settings
    else {
        return Err(CodexCatalogError::InvalidCatalog);
    };
    let allowed = settings.allowed_plan_ids();
    let plans: Vec<_> = allowed
        .iter()
        .filter_map(|plan_id| {
            publication
                .plans
                .iter()
                .find(|plan| plan.agent_plan_id() == plan_id)
                .cloned()
        })
        .collect();
    if plans.len() != allowed.len() {
        return Err(CodexCatalogError::InvalidCatalog);
    }
    let published = publication
        .published_agent_plans()
        .map_err(|_| CodexCatalogError::InvalidCatalog)?;
    let explicit_model = match default_selection {
        AgentModelDefaultSelectionV2::PreserveNative => None,
        AgentModelDefaultSelectionV2::FixedModel { client_model_id } => {
            Some(client_model_id.clone())
        }
        AgentModelDefaultSelectionV2::Plan { plan_id } => published
            .iter()
            .find(|plan| &plan.agent_plan_id == plan_id)
            .map(|plan| plan.model_alias.as_str().to_owned()),
    };
    let policy = CodexDefaultPolicy {
        explicit_model: explicit_model.as_deref(),
        // The client keeps its native subscription auth; subscription-only entries stay eligible.
        uses_codex_backend: true,
        allow_provider_model_fallback: false,
    };
    let scope = hiroute_integrations::CodexConfigurationScope::user_file(user_config.to_owned());
    if *native_model_mode == CodexNativeModelModeV2::HirouteOnly && fixed_models.is_empty() {
        return sample_codex_hiroute_only_catalog_plan(
            &scope,
            &plans,
            explicit_model
                .as_deref()
                .ok_or(CodexCatalogError::MissingDefault)?,
        );
    }
    let retained = settings
        .fixed_models()
        .iter()
        .map(|model| model.client_model_id.clone())
        .collect();
    sample_codex_catalog_plan(&scope, &plans, policy, baseline, Some(&retained))
}

impl LocalControlAdapter {
    pub(super) fn stage_settings_codex_catalog(
        &self,
        operation: &hiroute_domain::OperationV1,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<OwnedEffectV1> {
        let payload = decode_settings_codex_catalog(intent)?;
        let stores = self.stores_lock()?;
        let operation_id = &operation.operation_id;
        if operation.state != OperationState::ApplyingAgentArtifacts {
            return Err(conflict("codex.catalog.phase"));
        }
        let spec: hiroute_application_api::AgentSettingsSpecV2 =
            serde_json::from_value(operation.plan.spec().desired_state.clone())
                .map_err(|_| conflict("codex.catalog.spec"))?;
        let settings = match &spec.model {
            hiroute_application_api::AgentFacetIntent::Configure { settings } => settings,
            _ => return Err(conflict("codex.catalog.spec.model")),
        };
        if !matches!(settings, AgentModelSelectionV2::CodexDefault { .. }) {
            return Err(conflict("codex.catalog.spec.model"));
        }
        if operation.plan.spec().command_id != "agents.settings.apply"
            || spec.context_id != payload.context_id
            || !operation.plan.external().contains(intent)
        {
            return Err(conflict("codex.catalog.binding"));
        }
        let model_intent = operation
            .plan
            .external()
            .iter()
            .find(|candidate| is_settings_codex_model(candidate))
            .ok_or_else(|| conflict("codex.catalog.model.intent"))?;
        let model_payload = settings_codex_model_file_for_operation(operation, model_intent)?;
        let CodexModelFileAction::Configure {
            previous_operation,
            model_catalog: Some(expected),
            ..
        } = &model_payload.change
        else {
            return Err(conflict("codex.catalog.model.binding"));
        };
        if &payload.content_digest != expected {
            return Err(conflict("codex.catalog.digest.binding"));
        }
        let publication_intent = operation
            .plan
            .external()
            .iter()
            .find(|candidate| candidate.kind() == hiroute_domain::OwnedEffectKind::Publication)
            .ok_or_else(|| conflict("codex.catalog.publication.intent"))?
            .clone();
        let baseline = codex_catalog_baseline(
            stores.control(),
            &self.artifacts,
            previous_operation.as_ref(),
            model_intent.target(),
        )?;
        drop(stores);
        let publication = self
            .publication_record_from_operation(operation, &publication_intent)?
            .verify()
            .map_err(|_| conflict("codex.catalog.publication.verify"))?;
        // Recovery first: a staged artifact or protected restore bytes never re-derives the
        // catalog, so a changed user baseline cannot invalidate an already staged artifact.
        match restage_codex_catalog(&self.artifacts, operation_id, intent) {
            Ok(effect) => return Ok(effect),
            Err(error) if error.code != PortErrorCode::InvalidData => return Err(error),
            Err(_) => {}
        }
        let derived = codex_catalog_plan_for(
            &self.scanner.codex_user_config_target(),
            settings,
            &publication,
            &baseline,
        )
        .map_err(|_| conflict("codex.catalog.context_drift"))?;
        let producer_kind = match derived.producer.metadata_source {
            hiroute_integrations::CodexCatalogMetadataSourceV1::UserConfigured => {
                CodexCatalogProducerKindV1::UserConfigured
            }
            hiroute_integrations::CodexCatalogMetadataSourceV1::TargetCache => {
                CodexCatalogProducerKindV1::TargetCache
            }
            hiroute_integrations::CodexCatalogMetadataSourceV1::TargetBundled => {
                CodexCatalogProducerKindV1::TargetBundled
            }
            hiroute_integrations::CodexCatalogMetadataSourceV1::HirouteGenerated => {
                CodexCatalogProducerKindV1::HirouteGenerated
            }
        };
        if payload.source_revision != hiroute_integrations::CODEX_CATALOG_SOURCE_REVISION
            || payload.content_digest != derived.content_digest
            || payload.producer_kind != producer_kind
            || derived.producer.path.to_str() != Some(payload.producer_path.as_str())
            || payload.producer_content_digest != derived.producer.content_digest
            || payload.producer_context_digest != derived.producer.context_digest
            || payload.producer_dependency_digest != derived.producer.dependency_digest
        {
            return Err(conflict("codex.catalog.context_drift"));
        }
        stage_codex_catalog(&self.artifacts, operation_id, intent, &derived.selection)
    }

    pub(super) fn stage_settings_codex_model(
        &self,
        operation: &hiroute_domain::OperationV1,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<OwnedEffectV1> {
        let stores = self.stores_lock()?;
        let operation_id = &operation.operation_id;
        if operation.state != OperationState::ApplyingAgentArtifacts {
            return Err(conflict("codex.settings.phase"));
        }
        let payload = settings_codex_model_file_for_operation(operation, intent)?;
        match &payload.change {
            CodexModelFileAction::Configure {
                previous_operation,
                provider_id,
                endpoint,
                model,
                model_catalog,
            } => {
                let mutation = operation
                    .plan
                    .agent_access_grants()
                    .first()
                    .filter(|_| operation.plan.agent_access_grants().len() == 1)
                    .ok_or_else(|| conflict("codex.settings.grant"))?;
                let scope = mutation
                    .desired_scope()
                    .ok_or_else(|| conflict("codex.settings.scope"))?;
                if mutation.owner_scope() != operation.workspace_id.as_str()
                    || scope.connection_id() != format!("agent-connection/{}", payload.context_id)
                    || scope.protocol() != AgentIngressProtocolV1::Responses
                    || model
                        .as_deref()
                        .is_some_and(|name| !scope.model_grant().permits_name(name))
                {
                    return Err(conflict("codex.settings.grant.binding"));
                }
                let runtime = self
                    .managed_agent_runtime
                    .lock()
                    .map_err(|_| conflict("codex.settings.runtime.lock"))?;
                if runtime
                    .as_ref()
                    .is_none_or(|runtime| runtime.gateway_base_url != *endpoint)
                {
                    return Err(conflict("codex.settings.endpoint.changed"));
                }
                drop(runtime);
                let active = stores
                    .secrets()
                    .inspect_agent_access_grant(WorkspaceId::DEFAULT, scope.connection_id())?;
                let previous = if let Some(previous_operation) = previous_operation {
                    let previous = stores
                        .control()
                        .load_operation(previous_operation)?
                        .ok_or_else(|| conflict("codex.settings.previous.operation"))?;
                    if previous.workspace_id != operation.workspace_id
                        || previous.state != OperationState::Succeeded
                    {
                        return Err(conflict("codex.settings.previous.owner"));
                    }
                    let previous_intent = previous
                        .plan
                        .external()
                        .iter()
                        .find(|candidate| {
                            candidate.target() == intent.target()
                                && is_settings_codex_model(candidate)
                        })
                        .ok_or_else(|| conflict("codex.settings.previous.effect"))?
                        .clone();
                    let previous_payload =
                        settings_codex_model_file_for_operation(&previous, &previous_intent)?;
                    let CodexModelFileAction::Configure {
                        provider_id: previous_provider,
                        ..
                    } = previous_payload.change
                    else {
                        return Err(conflict("codex.settings.previous.action"));
                    };
                    let [previous_mutation] = previous.plan.agent_access_grants() else {
                        return Err(conflict("codex.settings.previous.grant"));
                    };
                    let previous_effect = previous
                        .step(OperationStepKind::ApplySecrets)
                        .effects
                        .iter()
                        .find(|effect| is_agent_access_grant_effect(effect))
                        .ok_or_else(|| conflict("codex.settings.previous.grant.effect"))?;
                    let previous_reference = AgentAccessGrantRefV1::from_ensure_effect(
                        previous_effect,
                        previous_mutation,
                    )
                    .map_err(|_| conflict("codex.settings.previous.grant.reference"))?;
                    if previous_provider != *provider_id
                        || active.as_ref() != Some(&previous_reference)
                        || previous_reference.generation() != mutation.expected_generation()
                    {
                        return Err(conflict("codex.settings.previous.binding"));
                    }
                    Some((previous, previous_intent))
                } else {
                    if active.is_some() {
                        return Err(conflict("codex.settings.previous.missing"));
                    }
                    None
                };
                let material = stores
                    .secrets()
                    .resolve_prepared_agent_access_grant(operation_id, mutation)?;
                drop(stores);
                let managed_aliases = scope
                    .model_grant()
                    .routes
                    .iter()
                    .filter(|(_, route)| matches!(route, AgentModelRouteV2::Plan { .. }))
                    .map(|(alias, _)| alias.clone())
                    .collect::<Vec<_>>();
                // The immutable catalog artifact this configuration points at; its absolute
                // path is resolved only through the trusted artifact port during rendering.
                let model_catalog_target = match model_catalog.as_ref() {
                    Some(digest) => Some(
                        operation
                            .plan
                            .external()
                            .iter()
                            .find(|candidate| {
                                decode_settings_codex_catalog(candidate)
                                    .is_ok_and(|catalog| &catalog.content_digest == digest)
                            })
                            .ok_or_else(|| conflict("codex.settings.catalog.binding"))?
                            .target()
                            .to_owned(),
                    ),
                    None => None,
                };
                let configuration = CodexFileConfiguration {
                    expected_content: &payload.expected_content,
                    selection: CodexSelectionTarget::Root,
                    provider_id,
                    endpoint,
                    model: model.as_deref(),
                    local_grant: &material,
                    model_catalog: model_catalog_target.as_deref(),
                    managed_aliases: &managed_aliases,
                };
                match &previous {
                    Some((previous, previous_intent)) => stage_codex_reconfiguration(
                        &self.artifacts,
                        operation_id,
                        intent,
                        &previous.operation_id,
                        previous_intent,
                        configuration,
                    ),
                    None => stage_codex_configuration(
                        &self.artifacts,
                        operation_id,
                        intent,
                        configuration,
                    ),
                }
            }
            CodexModelFileAction::Restore {
                original_operation,
                native_model,
            } => {
                let original = stores
                    .control()
                    .load_operation(original_operation)?
                    .ok_or_else(|| conflict("codex.settings.restore.original"))?;
                if original.workspace_id != operation.workspace_id
                    || original.state != OperationState::Succeeded
                {
                    return Err(conflict("codex.settings.restore.owner"));
                }
                let original_intent = original
                    .plan
                    .external()
                    .iter()
                    .find(|original| {
                        original.target() == intent.target() && is_settings_codex_model(original)
                    })
                    .ok_or_else(|| conflict("codex.settings.restore.effect"))?;
                let before = settings_codex_model_file_for_operation(&original, original_intent)?;
                if before.context_id != payload.context_id
                    || !matches!(before.change, CodexModelFileAction::Configure { .. })
                {
                    return Err(conflict("codex.settings.restore.context"));
                }
                drop(stores);
                match self.artifacts.observe_artifact(operation_id, intent)? {
                    EffectReconciliation::Staged(effect)
                    | EffectReconciliation::Applied(effect) => return Ok(effect),
                    EffectReconciliation::OwnershipLost(_) => {
                        return Err(conflict("codex.settings.restore.ownership"));
                    }
                    EffectReconciliation::Missing => {}
                }
                // Enforce the confirmed whole-file snapshot before field restoration. The store
                // checks ownership again at activation; unrelated later edits need a new Preview.
                let bytes = self.artifacts.read_native_target(intent.target())?;
                if CanonicalDigest::of_bytes(bytes.as_deref().map_or(&[], |bytes| bytes.as_slice()))
                    != payload.expected_content
                {
                    return Err(conflict("codex.settings.restore.changed"));
                }
                stage_codex_restoration(
                    &self.artifacts,
                    operation_id,
                    intent,
                    original_operation,
                    original_intent,
                    native_model.as_deref(),
                )
            }
        }
    }
}
fn conflict(context: &'static str) -> PortError {
    PortError::new(PortErrorCode::Conflict, context)
}

#[cfg(all(test, unix))]
#[path = "native_model_tests.rs"]
mod tests;
