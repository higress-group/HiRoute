//! Model facts bind the selected profile, publication, grant and bundled resources.
use super::*;
use hiroute_application::agent_connection::{
    HermesModelFileAction, NativePluginFileFacts, SettingsBlockReason,
    additional_model_provider_id, is_settings_hermes_model,
    settings_hermes_model_file_for_operation,
};
use hiroute_domain::{AgentCapability, AgentModelSelectionV2, AgentModelSurfaceV2};

impl LocalControlAdapter {
    pub(super) fn attach_hermes_model_facts(
        &self,
        spec: &AgentSettingsSpecV2,
        mut input: AgentSettingsPlanningInput,
    ) -> Result<AgentSettingsPlanningInput, ControlReadError> {
        if matches!(spec.model, AgentFacetIntent::Keep) {
            return Ok(input);
        }
        self.guard_additional_pending_model_change(None)
            .map_err(super::super::map_port)?;
        let target = AgentConnectionEffectRoleV1::ManagedConfiguration
            .settings_target_for(&input.subject)
            .map_err(|_| ControlReadError::Corrupt)?;
        let before_fingerprint = self
            .artifacts
            .current_external_fingerprint(&target)
            .map_err(super::super::map_port)?;
        let bytes = self
            .artifacts
            .read_native_target(&target)
            .map_err(super::super::map_port)?;
        let expected_content =
            CanonicalDigest::of_bytes(bytes.as_deref().map_or(&[], Vec::as_slice));
        drop(bytes);
        if before_fingerprint
            != self
                .artifacts
                .current_external_fingerprint(&target)
                .map_err(super::super::map_port)?
        {
            return Err(ControlReadError::SnapshotChanged);
        }
        let stores = self.stores_lock().map_err(super::super::map_port)?;
        let workspace = WorkspaceId::default();
        let publication = stores
            .control()
            .active_publication(&workspace)
            .map_err(super::super::map_port)?
            .ok_or(ControlReadError::NotFound)?;
        let active = publication
            .verify()
            .map_err(|_| ControlReadError::Corrupt)?;
        let connection_id = format!("agent-connection/{}", spec.context_id);
        let current_grant = stores
            .secrets()
            .inspect_agent_access_grant(workspace.as_str(), &connection_id)
            .map_err(super::super::map_port)?;
        let token_input_fingerprint = match &spec.access_token {
            AgentAccessTokenIntentV1::Keep => None,
            AgentAccessTokenIntentV1::Regenerate if current_grant.is_some() => None,
            AgentAccessTokenIntentV1::Set { input_slot } if current_grant.is_some() => {
                let inputs = self
                    .agent_token_inputs
                    .lock()
                    .map_err(|_| ControlReadError::Unavailable)?;
                let secret = inputs.get(input_slot).ok_or(ControlReadError::NotFound)?;
                if !hiroute_domain::valid_user_agent_token(secret.expose()) {
                    return Err(ControlReadError::Denied);
                }
                Some(
                    stores
                        .secrets()
                        .fingerprint(secret)
                        .map_err(super::super::map_port)?,
                )
            }
            _ => return Err(ControlReadError::Denied),
        };
        let mut previous = None;
        if let Some(reference) = &current_grant {
            for operation in stores
                .control()
                .succeeded_agent_operations_for_kind(&workspace, "ApplyAgentConnectionChange")
                .map_err(super::super::map_port)?
            {
                if operation.plan.spec().resource_id.as_deref() != Some(&spec.context_id) {
                    continue;
                }
                let Some(intent) = operation
                    .plan
                    .external()
                    .iter()
                    .find(|i| is_settings_hermes_model(i))
                    .cloned()
                else {
                    continue;
                };
                if !matches!(
                    settings_hermes_model_file_for_operation(&operation, &intent)
                        .map_err(super::super::map_port)?
                        .change,
                    HermesModelFileAction::Configure { .. }
                ) {
                    return Err(ControlReadError::Corrupt);
                }
                let [mutation] = operation.plan.agent_access_grants() else {
                    return Err(ControlReadError::Corrupt);
                };
                let effect = operation
                    .grant_effects()
                    .iter()
                    .find(|effect| is_agent_access_grant_effect(effect))
                    .ok_or(ControlReadError::Corrupt)?;
                let owned = AgentAccessGrantRefV1::from_ensure_effect(effect, mutation)
                    .map_err(|_| ControlReadError::Corrupt)?;
                if &owned == reference {
                    previous = Some((operation, intent));
                    break;
                }
            }
            if previous.is_none() {
                return Err(ControlReadError::Corrupt);
            }
        }
        let generation = stores
            .secrets()
            .agent_access_grant_generation(WorkspaceId::DEFAULT, &connection_id)
            .map_err(super::super::map_port)?;
        drop(stores);
        let provider_id = additional_model_provider_id(&spec.context_id);
        let endpoint = self
            .managed_agent_runtime
            .lock()
            .map_err(|_| ControlReadError::Unavailable)?
            .as_ref()
            .map(|r| r.gateway_base_url.clone())
            .ok_or(ControlReadError::Unavailable)?;
        let mut restore = None;
        if let (Some((operation, _)), Some(reference)) = (&previous, &current_grant) {
            input.facts.restore_points.insert(
                codex_model_restore_point_ref(&operation.operation_id),
                AgentSettingsFacet::Model,
            );
            restore = Some((operation.operation_id.clone(), reference.clone()));
        }
        let (model, validation) = match &spec.model {
            AgentFacetIntent::Configure {
                settings: settings @ AgentModelSelectionV2::HermesDefault { protocol, .. },
            } => {
                let grant = hiroute_domain::AgentModelGrantV2::derive(
                    *protocol,
                    settings,
                    &active,
                    &BTreeMap::new(),
                )
                .map_err(|_| ControlReadError::Denied)?;
                let mut models =
                    super::super::native_additional_model::additional_models_for_grant(
                        hiroute_domain::AgentKindV1::Hermes,
                        &active,
                        &grant,
                    )
                    .map_err(super::super::map_port)?;
                if models.len() != 1 {
                    return Err(ControlReadError::Corrupt);
                }
                let model = models.remove(0);
                let validation = hiroute_integrations::validate_hermes_configuration(
                    &self.artifacts,
                    &target,
                    &provider_id,
                    &endpoint,
                    &model,
                    previous
                        .as_ref()
                        .map(|(op, intent)| (&op.operation_id, intent)),
                );
                (model, validation)
            }
            AgentFacetIntent::Restore { restore_point_ref } => {
                let (original, intent) = previous
                    .as_ref()
                    .filter(|(op, _)| {
                        codex_model_restore_point_ref(&op.operation_id) == *restore_point_ref
                    })
                    .ok_or(ControlReadError::Denied)?;
                let HermesModelFileAction::Configure { model, .. } =
                    settings_hermes_model_file_for_operation(original, intent)
                        .map_err(super::super::map_port)?
                        .change
                else {
                    return Err(ControlReadError::Corrupt);
                };
                (
                    model,
                    hiroute_integrations::validate_hermes_restoration(
                        &self.artifacts,
                        &target,
                        &original.operation_id,
                        intent,
                    ),
                )
            }
            _ => return Err(ControlReadError::Denied),
        };
        let mut conflict = native_conflict(validation)?;
        let mut plugins = Vec::new();
        for (role, content) in [
            (
                AgentConnectionEffectRoleV1::HermesPluginModule,
                hiroute_integrations::HERMES_PLUGIN_MODULE,
            ),
            (
                AgentConnectionEffectRoleV1::HermesPluginManifest,
                hiroute_integrations::HERMES_PLUGIN_MANIFEST,
            ),
        ] {
            let file_target = role
                .settings_target_for(&input.subject)
                .map_err(|_| ControlReadError::Corrupt)?;
            let fingerprint = self
                .artifacts
                .current_external_fingerprint(&file_target)
                .map_err(super::super::map_port)?;
            let bytes = self
                .artifacts
                .read_native_target(&file_target)
                .map_err(super::super::map_port)?;
            let digest = CanonicalDigest::of_bytes(bytes.as_deref().map_or(&[], Vec::as_slice));
            drop(bytes);
            if fingerprint
                != self
                    .artifacts
                    .current_external_fingerprint(&file_target)
                    .map_err(super::super::map_port)?
            {
                return Err(ControlReadError::SnapshotChanged);
            }
            let original = previous.as_ref().map(|(op, _)| {
                let intent = op.plan.external().iter().find(|i| i.target() == file_target).ok_or(ControlReadError::Corrupt)?;
                hiroute_application::agent_connection::settings_native_plugin_file_for_operation(op, intent).map_err(super::super::map_port)?;
                Ok::<_, ControlReadError>((&op.operation_id, intent))
            }).transpose()?;
            let result = match &spec.model {
                AgentFacetIntent::Configure { .. } => {
                    hiroute_integrations::validate_native_plugin_configuration(
                        &self.artifacts,
                        &file_target,
                        content.as_bytes(),
                        original,
                    )
                }
                AgentFacetIntent::Restore { .. } => {
                    let (op, intent) = original.ok_or(ControlReadError::Corrupt)?;
                    hiroute_integrations::validate_native_plugin_restoration(
                        &self.artifacts,
                        &file_target,
                        op,
                        intent,
                    )
                }
                _ => unreachable!(),
            };
            conflict = conflict.or(native_conflict(result)?);
            plugins.push(NativePluginFileFacts {
                role,
                before_fingerprint: fingerprint,
                expected_content: digest,
                content,
            });
        }
        let digest = CanonicalDigest::of(&serde_json::json!({
            "schema":"hermes-model-facts/v1", "skill":input.facts.dependency_digest,
            "target":input.facts.native_target,"content":expected_content,"fingerprint":before_fingerprint,
            "publication":publication.digest,"grant":current_grant,"generation":generation,"token":token_input_fingerprint,
            "provider":provider_id,"endpoint":endpoint,"model":model,"conflict":conflict,
            "previous":previous.as_ref().map(|(op,_)| &op.operation_id),"restore":restore,
            "plugins":plugins.iter().map(|p| (&p.role,&p.before_fingerprint,&p.expected_content,CanonicalDigest::of_bytes(p.content.as_bytes()))).collect::<Vec<_>>(),
        })).map_err(|_| ControlReadError::Corrupt)?;
        let old = input.facts.dependency_digest.clone();
        let mut proofs = [
            AgentCapability::EffectiveConfiguration,
            AgentCapability::AtomicManagedReplace,
            AgentCapability::IngressAuthentication,
            AgentCapability::ModelCatalog,
            AgentCapability::SkillLoading,
            AgentCapability::TrustedCliExecution,
            AgentCapability::IsolatedVerification,
        ]
        .into_iter()
        .filter_map(|kind| input.facts.capabilities.get(kind).cloned())
        .collect::<Vec<_>>();
        for proof in &mut proofs {
            if proof.dependency_digest == old {
                proof.dependency_digest = digest.clone();
            }
        }
        input.facts.capabilities =
            AgentCapabilitySet::new(proofs).map_err(|_| ControlReadError::Corrupt)?;
        input.facts.dependency_digest = digest;
        input.facts.model = Some(SettingsModelFacts {
            common: SettingsModelCommonFacts {
                ingress: model.protocol,
                available_surfaces: [AgentModelSurfaceV2::HermesCli].into(),
                model_publication: Some(active),
                login_item_required: false,
                login_item_removal_required: false,
                fixed_candidate_facts: Vec::new(),
            },
            native: SettingsModelNativeFacts::Hermes(SettingsAdditionalModelFacts {
                model_conflict: conflict,
            }),
        });
        input.model_file = Some(SettingsModelFileFacts {
            expected_content,
            before_fingerprint,
            publication_digest: Some(publication.digest),
            expected_grant_generation: generation,
            token_input_fingerprint,
            active_configuration: previous.as_ref().map(|(op, _)| op.operation_id.clone()),
            restore,
            target: SettingsModelTargetFacts::Hermes {
                provider_id,
                endpoint,
                model,
                plugins,
            },
        });
        Ok(input)
    }
}

fn native_conflict(
    result: hiroute_domain::PortResult<()>,
) -> Result<Option<SettingsBlockReason>, ControlReadError> {
    match result {
        Ok(()) => Ok(None),
        Err(error) if error.code == hiroute_domain::PortErrorCode::Conflict => {
            Ok(Some(SettingsBlockReason::HermesModelFileConflict))
        }
        Err(error) => Err(super::super::map_port(error)),
    }
}
