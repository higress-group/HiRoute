//! Optional Qoder model facts. Pure collaboration never enters this file reader.
use super::*;
use hiroute_application::agent_connection::{
    AdditionalModelFileAction, additional_model_provider_id,
    settings_additional_model_file_for_operation,
};
use hiroute_domain::{
    AgentCapability, AgentIngressProtocolV1, AgentModelSelectionV2, AgentModelSurfaceV2,
    CapabilityEvidence, CapabilityState,
};

impl LocalControlAdapter {
    pub(super) fn attach_additional_model_facts(
        &self,
        spec: &AgentSettingsSpecV2,
        class: SettingsAgentClass,
        mut input: AgentSettingsPlanningInput,
    ) -> Result<AgentSettingsPlanningInput, ControlReadError> {
        // A model-capable installation does not make unrelated native settings a dependency
        // of the Skill. The ordinary planner still rejects token changes without Configure.
        if matches!(spec.model, AgentFacetIntent::Keep) {
            return Ok(input);
        }
        if spec.restore_native_model.is_some() {
            return Ok(input);
        }
        if let AgentFacetIntent::Configure { settings } = &spec.model
            && !matches!(
                settings,
                AgentModelSelectionV2::QoderAdditional { .. }
                    | AgentModelSelectionV2::PiAdditional { .. }
                    | AgentModelSelectionV2::DshAdditional { .. }
            )
        {
            return Ok(input);
        }
        let kind = if class == SettingsAgentClass::Dsh {
            hiroute_domain::AgentKindV1::DeepseekHarness
        } else if class == SettingsAgentClass::Pi {
            hiroute_domain::AgentKindV1::Pi
        } else {
            hiroute_domain::AgentKindV1::Qoder
        };
        // Restore remains available even after the installed SDK becomes incompatible.
        #[cfg(unix)]
        if kind == hiroute_domain::AgentKindV1::Pi
            && matches!(spec.model, AgentFacetIntent::Configure { .. })
        {
            self.scanner
                .check_pi_model_configuration()
                .map_err(|_| ControlReadError::Unavailable)?;
        }
        #[cfg(unix)]
        if kind == hiroute_domain::AgentKindV1::DeepseekHarness
            && matches!(spec.model, AgentFacetIntent::Configure { .. })
        {
            self.scanner
                .check_dsh_model_configuration()
                .map_err(|_| ControlReadError::Unavailable)?;
        }
        let pi_settings = if kind == hiroute_domain::AgentKindV1::Pi {
            Some(
                self.scanner
                    .pi_default_model()
                    .map_err(|_| ControlReadError::Denied)?,
            )
        } else {
            None
        };
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
            CanonicalDigest::of_bytes(bytes.as_deref().map_or(&[][..], Vec::as_slice));
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
        let current_grant = stores
            .secrets()
            .inspect_agent_access_grant(
                workspace.as_str(),
                &format!("agent-connection/{}", spec.context_id),
            )
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
        let previous = if let Some(reference) = &current_grant {
            let mut found = None;
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
                    .find(|intent| {
                        super::super::native_additional_model::is_settings_additional_model(intent)
                    })
                    .cloned()
                else {
                    continue;
                };
                let payload = settings_additional_model_file_for_operation(&operation, &intent)
                    .map_err(super::super::map_port)?;
                if !matches!(payload.change, AdditionalModelFileAction::Configure { .. }) {
                    return Err(ControlReadError::Corrupt);
                }
                let [mutation] = operation.plan.agent_access_grants() else {
                    return Err(ControlReadError::Corrupt);
                };
                let effect = operation
                    .step(hiroute_domain::OperationStepKind::ApplySecrets)
                    .effects
                    .iter()
                    .find(|effect| is_agent_access_grant_effect(effect))
                    .ok_or(ControlReadError::Corrupt)?;
                let original = AgentAccessGrantRefV1::from_ensure_effect(effect, mutation)
                    .map_err(|_| ControlReadError::Corrupt)?;
                if &original == reference {
                    found = Some((operation, intent));
                    break;
                }
            }
            Some(found.ok_or(ControlReadError::Corrupt)?)
        } else {
            None
        };
        let generation = stores
            .secrets()
            .agent_access_grant_generation(
                WorkspaceId::DEFAULT,
                &format!("agent-connection/{}", spec.context_id),
            )
            .map_err(super::super::map_port)?;
        drop(stores);
        let provider_id = additional_model_provider_id(&spec.context_id);
        let endpoint = self
            .managed_agent_runtime
            .lock()
            .map_err(|_| ControlReadError::Unavailable)?
            .as_ref()
            .map(|runtime| runtime.gateway_base_url.clone())
            .ok_or(ControlReadError::Unavailable)?;
        let endpoint =
            super::super::native_additional_model::additional_model_endpoint(kind, &endpoint)
                .map_err(super::super::map_port)?;
        let mut restore = None;
        if let (Some((operation, _)), Some(reference)) = (&previous, &current_grant) {
            input.facts.restore_points.insert(
                codex_model_restore_point_ref(&operation.operation_id),
                AgentSettingsFacet::Model,
            );
            restore = Some((operation.operation_id.clone(), reference.clone()));
        }
        let protocol = AgentIngressProtocolV1::Responses;
        let qoder_model_conflict;
        let models = match &spec.model {
            AgentFacetIntent::Configure { settings } => {
                let grant = hiroute_domain::AgentModelGrantV2::derive(
                    protocol,
                    settings,
                    &active,
                    &BTreeMap::new(),
                )
                .map_err(|_| ControlReadError::Denied)?;
                let models = super::super::native_additional_model::additional_models_for_grant(
                    kind, &active, &grant,
                )
                .map_err(super::super::map_port)?;
                qoder_model_conflict = qoder_model_validation(
                    hiroute_integrations::validate_additional_configuration(
                        &self.artifacts,
                        kind,
                        &target,
                        &provider_id,
                        &endpoint,
                        &models,
                        previous
                            .as_ref()
                            .map(|(op, intent)| (&op.operation_id, intent)),
                    ),
                )?;
                models
            }
            AgentFacetIntent::Restore { restore_point_ref } => {
                let (original, intent) = previous
                    .as_ref()
                    .filter(|(op, _)| {
                        codex_model_restore_point_ref(&op.operation_id) == *restore_point_ref
                    })
                    .ok_or(ControlReadError::Denied)?;
                qoder_model_conflict =
                    qoder_model_validation(hiroute_integrations::validate_additional_restoration(
                        &self.artifacts,
                        kind,
                        &target,
                        &original.operation_id,
                        intent,
                    ))?;
                Vec::new()
            }
            AgentFacetIntent::Keep => unreachable!(),
        };
        let dsh_default_in_use = kind == hiroute_domain::AgentKindV1::DeepseekHarness
            && self
                .scanner
                .dsh_web_default_in_use(&provider_id, &models)
                .map_err(|_| ControlReadError::Denied)?;
        let qoder_model_conflict = if dsh_default_in_use
            || pi_settings
                .as_ref()
                .is_some_and(|settings| settings.removes_default(&provider_id, &models))
            || (kind != hiroute_domain::AgentKindV1::Qoder
                && qoder_model_conflict
                    == Some(
                    hiroute_application::agent_connection::SettingsBlockReason::QoderDefaultInUse,
                ))
        {
            Some(hiroute_application::agent_connection::SettingsBlockReason::AdditionalDefaultInUse)
        } else {
            qoder_model_conflict
        };
        let dependency_digest = CanonicalDigest::of(&serde_json::json!({
            "schema":"qoder-model-facts/v1", "collaboration":input.facts.dependency_digest,
            "content":expected_content,"fingerprint":before_fingerprint,"publication":publication.digest,
            "grant":current_grant,"token":token_input_fingerprint,"provider":provider_id,"endpoint":endpoint,
            "conflict":qoder_model_conflict,"models":models,"kind":kind,"pi_settings":pi_settings,"previous":previous.as_ref().map(|(op,_)| &op.operation_id),"restore":restore,
        })).map_err(|_| ControlReadError::Corrupt)?;
        let old_digest = input.facts.dependency_digest.clone();
        let mut capabilities = [
            AgentCapability::EffectiveConfiguration,
            AgentCapability::AtomicManagedReplace,
            AgentCapability::IngressAuthentication,
            AgentCapability::ModelCatalog,
            AgentCapability::SkillLoading,
            AgentCapability::TrustedCliExecution,
            AgentCapability::IsolatedVerification,
        ]
        .iter()
        .filter_map(|kind| input.facts.capabilities.get(*kind).cloned())
        .map(|mut proof| {
            if proof.dependency_digest == old_digest {
                proof.dependency_digest = dependency_digest.clone();
            }
            proof
        })
        .collect::<Vec<_>>();
        for capability in [
            AgentCapability::EffectiveConfiguration,
            AgentCapability::AtomicManagedReplace,
        ] {
            capabilities.retain(|proof| proof.capability != capability);
            capabilities.push(CapabilityEvidence {
                capability,
                state: CapabilityState::Proven,
                adapter_contract: match kind {
                    hiroute_domain::AgentKindV1::Pi => "hiroute.pi-additional-model/v1",
                    hiroute_domain::AgentKindV1::DeepseekHarness => {
                        "hiroute.dsh-additional-model/v1"
                    }
                    _ => "hiroute.qoder-additional-model/v1",
                }
                .into(),
                observed_at_unix_ms: self.now_ms()?.max(1) as u64,
                dependency_digest: dependency_digest.clone(),
                reason: None,
            });
        }
        input.facts.capabilities =
            AgentCapabilitySet::new(capabilities).map_err(|_| ControlReadError::Corrupt)?;
        input.facts.dependency_digest = dependency_digest;
        input.facts.model = Some(SettingsModelFacts {
            common: SettingsModelCommonFacts {
                ingress: protocol,
                available_surfaces: [if kind == hiroute_domain::AgentKindV1::DeepseekHarness {
                    AgentModelSurfaceV2::DshCli
                } else if kind == hiroute_domain::AgentKindV1::Pi {
                    AgentModelSurfaceV2::PiCli
                } else {
                    AgentModelSurfaceV2::QoderCli
                }]
                .into(),
                model_publication: Some(active),
                login_item_required: false,
                login_item_removal_required: false,
                fixed_candidate_facts: Vec::new(),
            },
            native: SettingsModelNativeFacts::Additional(SettingsAdditionalModelFacts {
                model_conflict: qoder_model_conflict,
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
            target: SettingsModelTargetFacts::Additional {
                kind,
                pi_settings_content: pi_settings.map(|settings| settings.content_digest),
                provider_id,
                endpoint,
                models,
            },
        });
        Ok(input)
    }
}

fn qoder_model_validation(
    result: hiroute_domain::PortResult<()>,
) -> Result<Option<hiroute_application::agent_connection::SettingsBlockReason>, ControlReadError> {
    use hiroute_application::agent_connection::SettingsBlockReason;
    match result {
        Ok(()) => Ok(None),
        Err(error)
            if error.code == hiroute_domain::PortErrorCode::Conflict
                && error.context == "qoder.default.in-use" =>
        {
            Ok(Some(SettingsBlockReason::QoderDefaultInUse))
        }
        Err(error)
            if error.code == hiroute_domain::PortErrorCode::Conflict
                && error.context == "qoder.native.fields" =>
        {
            Ok(Some(SettingsBlockReason::QoderModelFileConflict))
        }
        Err(error) => Err(super::super::map_port(error)),
    }
}
