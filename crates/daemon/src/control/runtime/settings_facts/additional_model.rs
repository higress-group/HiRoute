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
                    | AgentModelSelectionV2::OpenCodeAdditional { .. }
            )
        {
            return Ok(input);
        }
        let kind = if class == SettingsAgentClass::OpenCode {
            hiroute_domain::AgentKindV1::OpenCode
        } else if matches!(
            class,
            SettingsAgentClass::Dsh | SettingsAgentClass::DshDesktop
        ) {
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
            let found = stores
                .control()
                .find_verified_agent_operation(
                    &workspace,
                    &["ApplyAgentConnectionChange"],
                    |operation| {
                        let invalid = || {
                            hiroute_domain::PortError::new(
                                hiroute_domain::PortErrorCode::Corrupt,
                                "agent.model.current-grant",
                            )
                        };
                        if operation.plan.spec().resource_id.as_deref() != Some(&spec.context_id) {
                            return Ok(false);
                        }
                        let Some(intent) = operation
                            .plan
                            .external()
                            .iter()
                            .find(|intent| {
                                super::super::native_additional_model::is_settings_additional_model(
                                    intent,
                                )
                            })
                            .cloned()
                        else {
                            return Ok(false);
                        };
                        let payload =
                            settings_additional_model_file_for_operation(operation, &intent)?;
                        if !matches!(payload.change, AdditionalModelFileAction::Configure { .. }) {
                            return Err(invalid());
                        }
                        let [mutation] = operation.plan.agent_access_grants() else {
                            return Err(invalid());
                        };
                        let effect = operation
                            .grant_effects()
                            .iter()
                            .find(|effect| is_agent_access_grant_effect(effect))
                            .ok_or_else(invalid)?;
                        let original = AgentAccessGrantRefV1::from_ensure_effect(effect, mutation)
                            .map_err(|_| invalid())?;
                        Ok(&original == reference)
                    },
                )
                .map_err(super::super::map_port)?
                .ok_or(ControlReadError::Corrupt)?;
            let intent = found
                .plan
                .external()
                .iter()
                .find(|intent| {
                    super::super::native_additional_model::is_settings_additional_model(intent)
                })
                .cloned()
                .ok_or(ControlReadError::Corrupt)?;
            Some((found, intent))
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
        let (additional_dependencies, opencode_default_in_use) =
            if kind == hiroute_domain::AgentKindV1::OpenCode {
                let (digest, in_use) = self
                    .scanner
                    .opencode_model_facts(&provider_id, &models)
                    .map_err(|_| ControlReadError::SnapshotChanged)?;
                (Some(digest), in_use)
            } else {
                (None, false)
            };
        let dsh_default_in_use = kind == hiroute_domain::AgentKindV1::DeepseekHarness
            && self
                .scanner
                .dsh_web_default_in_use(&provider_id, &models)
                .map_err(|_| ControlReadError::Denied)?;
        let mut qoder_model_conflict = if opencode_default_in_use || dsh_default_in_use
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
        let routing_adapter =
            if let Some(content) = hiroute_integrations::native_routing_adapter(kind) {
                use hiroute_application::agent_connection::NativePluginFileFacts;
                let role = AgentConnectionEffectRoleV1::NativeRoutingAdapter;
                let file_target = role
                    .settings_target_for(&input.subject)
                    .map_err(|_| ControlReadError::Corrupt)?;
                let before_fingerprint = self
                    .artifacts
                    .current_external_fingerprint(&file_target)
                    .map_err(super::super::map_port)?;
                let bytes = self
                    .artifacts
                    .read_native_target(&file_target)
                    .map_err(super::super::map_port)?;
                let expected_content =
                    CanonicalDigest::of_bytes(bytes.as_deref().map_or(&[], Vec::as_slice));
                let old = previous.as_ref().and_then(|(operation, _)| {
                    operation
                        .plan
                        .external()
                        .iter()
                        .find(|i| i.effect_id() == "agent-connection-native-routing-adapter")
                        .map(|i| (&operation.operation_id, i))
                });
                if matches!(&spec.model, AgentFacetIntent::Configure { settings }
                    if !settings.installs_native_routing_metadata())
                {
                    // Do not strand an owned adapter by dropping its effect from the next
                    // Operation. Normal Restore releases its journal before this explicit
                    // no-plugin configuration. A foreign/borrowed file is never deleted.
                    use hiroute_application::agent_connection::SettingsBlockReason;
                    let blocker = if old.is_some() {
                        Some(SettingsBlockReason::NativeRoutingAdapterRestoreRequired)
                    } else if bytes.is_some() {
                        Some(SettingsBlockReason::NativeRoutingAdapterExternalResource)
                    } else {
                        None
                    };
                    qoder_model_conflict = qoder_model_conflict.or(blocker);
                    None
                } else if matches!(spec.model, AgentFacetIntent::Configure { .. }) {
                    hiroute_integrations::validate_native_plugin_configuration(
                        &self.artifacts,
                        &file_target,
                        content.as_bytes(),
                        old,
                    )
                    .map_err(super::super::map_port)?;
                    Some(NativePluginFileFacts {
                        role,
                        before_fingerprint,
                        expected_content,
                        content,
                    })
                } else if let Some((operation, intent)) = old {
                    hiroute_integrations::validate_native_plugin_restoration(
                        &self.artifacts,
                        &file_target,
                        operation,
                        intent,
                    )
                    .map_err(super::super::map_port)?;
                    Some(NativePluginFileFacts {
                        role,
                        before_fingerprint,
                        expected_content,
                        content,
                    })
                } else {
                    None
                }
            } else {
                None
            };
        let dependency_facts = serde_json::json!({
            "schema":"qoder-model-facts/v1", "routing_adapter":routing_adapter.as_ref().map(|r| (&r.expected_content, &r.before_fingerprint, CanonicalDigest::of_bytes(r.content.as_bytes()))), "collaboration":input.facts.dependency_digest,
            "content":expected_content,"fingerprint":before_fingerprint,"publication":publication.digest,
            "grant":current_grant,"token":token_input_fingerprint,"provider":provider_id,"endpoint":endpoint,
            "conflict":qoder_model_conflict,"models":models,"additional_dependencies":additional_dependencies,"kind":kind,"pi_settings":pi_settings,"previous":previous.as_ref().map(|(op,_)| &op.operation_id),"restore":restore,
        });
        #[cfg(test)]
        diagnostics::record("additional_model", &dependency_facts);
        let dependency_digest =
            CanonicalDigest::of(&dependency_facts).map_err(|_| ControlReadError::Corrupt)?;
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
                    hiroute_domain::AgentKindV1::OpenCode => "hiroute.opencode-additional-model/v1",
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
                available_surfaces: [if kind == hiroute_domain::AgentKindV1::OpenCode {
                    AgentModelSurfaceV2::OpenCodeCli
                } else if class == SettingsAgentClass::DshDesktop {
                    AgentModelSurfaceV2::DshDesktop
                } else if kind == hiroute_domain::AgentKindV1::DeepseekHarness {
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
                additional_dependencies,
                routing_adapter,
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
