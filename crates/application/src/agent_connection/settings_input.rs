//! Settings facts transported from the real backend to the existing typed planner.
use super::*;
use hiroute_domain::{
    AgentAccessGrantMaterialActionV1, AgentAccessGrantMutationV1, AgentAccessGrantRefV1,
    AgentAccessGrantScopeV1, AgentAccessTokenIntentV1, AgentConnectionTransactionSubjectV1,
    AgentFacetIntent, AgentIngressProtocolV1, AgentModelSelectionV2, CanonicalDigest, OperationId,
    OperationValidationError, RevisionSetV1, SupportedAgentInstallationV1, TransactionPlanV1,
    WorkspaceId, settings_model_publication_intent,
};
use serde_json::json;
use std::path::Path;

pub struct AgentSettingsPlanningInput {
    pub collaboration_state: serde_json::Value,
    pub facts: AgentSettingsFacts,
    pub expected_revisions: RevisionSetV1,
    pub subject: AgentConnectionTransactionSubjectV1,
    pub model_file: Option<SettingsModelFileFacts>,
    pub skill_file: SettingsSkillFileFacts,
    /// The independent second resource. The primary slot retains accepted legacy semantics.
    pub additional_skill_file: Option<SettingsSkillFileFacts>,
}

/// Non-secret backend facts, all included in the accepted dependency digest.
pub struct SettingsModelFileFacts {
    pub expected_content: CanonicalDigest,
    pub before_fingerprint: Option<CanonicalDigest>,
    /// Collaboration-only settings remain valid before the first publication. Model changes
    /// still require and bind the exact active publication when their effect is sealed.
    pub publication_digest: Option<CanonicalDigest>,
    pub expected_grant_generation: u64,
    pub token_input_fingerprint: Option<CanonicalDigest>,
    /// The exact succeeded configuration that owns the currently active grant, if any.
    pub active_configuration: Option<OperationId>,
    pub restore: Option<(OperationId, AgentAccessGrantRefV1)>,
    pub target: SettingsModelTargetFacts,
}

/// Each main Agent keeps one registered managed-configuration target.  The current target is
/// selected only by the daemon's registered scanner/profile join, never by a public request.
#[allow(clippy::large_enum_variant)] // These short-lived planner facts stay typed and allocation-free.
pub enum SettingsModelTargetFacts {
    Hermes {
        provider_id: String,
        endpoint: String,
        model: hiroute_domain::AdditionalAgentModelV1,
        plugins: Vec<super::NativePluginFileFacts>,
    },
    Codex {
        provider_id: String,
        endpoint: String,
    },
    Claude(SettingsClaudeModelFacts),
    Additional {
        kind: hiroute_domain::AgentKindV1,
        pi_settings_content: Option<CanonicalDigest>,
        additional_dependencies: Option<CanonicalDigest>,
        routing_adapter: Option<super::NativePluginFileFacts>,
        provider_id: String,
        endpoint: String,
        models: Vec<hiroute_domain::AdditionalAgentModelV1>,
    },
}

pub struct SettingsClaudeModelFacts {
    pub installation: SupportedAgentInstallationV1,
    pub executable: Option<String>,
    pub gateway_base_url: String,
    pub trusted_hiroute_executable: String,
    pub user_document: hiroute_domain::AgentConfigDocumentV1,
}

/// Shared-file facts for one registered Agent-owned Skill root.  The template is bundled product
/// content, not a scanner result or WebView payload.
pub struct SettingsSkillFileFacts {
    pub root_ref: String,
    pub target: String,
    pub before: Option<ManagedCollaborationSkill>,
    pub observed_file: Option<CanonicalDigest>,
    pub before_fingerprint: Option<CanonicalDigest>,
    pub template: CollaborationSkillTemplate,
}

impl AgentSettingsPlanningInput {
    pub fn seal_settings(
        &self,
        confirmed: &ConfirmedAgentSettings,
    ) -> Result<TransactionPlanV1, OperationValidationError> {
        let invalid = || OperationValidationError::UnregisteredEffectPlan;
        let preview = confirmed.preview();
        if preview.dependency_digest != self.facts.dependency_digest
            || preview.spec.context_id != self.facts.context_id
        {
            return Err(invalid());
        }
        if self.facts.model.is_none()
            && (!matches!(preview.spec.model, AgentFacetIntent::Keep)
                || !matches!(preview.spec.access_token, AgentAccessTokenIntentV1::Keep)
                || preview.model_grant.is_some())
        {
            return Err(invalid());
        }
        let (model_mutations, model_action, restore_grant) = match &preview.spec.model {
            AgentFacetIntent::Configure { settings } => {
                let file = self.model_file.as_ref().ok_or_else(invalid)?;
                let model = self.facts.model.as_ref().ok_or_else(invalid)?;
                let grant = preview.model_grant.as_ref().ok_or_else(invalid)?;
                let scope = AgentAccessGrantScopeV1::new(
                    format!("agent-connection/{}", self.facts.context_id),
                    grant.clone(),
                )?;
                let action = match &file.target {
                    SettingsModelTargetFacts::Hermes {
                        provider_id,
                        endpoint,
                        model,
                        ..
                    } => {
                        if !matches!(settings, AgentModelSelectionV2::HermesDefault { .. }) {
                            return Err(invalid());
                        }
                        SettingsModelAction::Hermes(super::HermesModelFileAction::Configure {
                            previous_operation: file.active_configuration.clone(),
                            provider_id: provider_id.clone(),
                            endpoint: endpoint.clone(),
                            model: model.clone(),
                        })
                    }
                    SettingsModelTargetFacts::Codex {
                        provider_id,
                        endpoint,
                    } => SettingsModelAction::Codex(CodexModelFileAction::Configure {
                        previous_operation: file.active_configuration.clone(),
                        provider_id: provider_id.clone(),
                        endpoint: endpoint.clone(),
                        model: grant
                            .codex_default_override(settings)
                            .map_err(|_| invalid())?,
                        model_catalog: model
                            .codex()
                            .and_then(|facts| facts.model_catalog.as_ref())
                            .map(|catalog| catalog.content_digest.clone()),
                    }),
                    SettingsModelTargetFacts::Additional {
                        kind,
                        pi_settings_content,
                        additional_dependencies,
                        routing_adapter: _,
                        provider_id,
                        endpoint,
                        models,
                    } => {
                        if !matches!(
                            settings,
                            AgentModelSelectionV2::QoderAdditional { .. }
                                | AgentModelSelectionV2::PiAdditional { .. }
                                | AgentModelSelectionV2::DshAdditional { .. }
                                | AgentModelSelectionV2::OpenCodeAdditional { .. }
                        ) || matches!(settings, AgentModelSelectionV2::PiAdditional { .. })
                            != (*kind == hiroute_domain::AgentKindV1::Pi)
                            || matches!(settings, AgentModelSelectionV2::DshAdditional { .. })
                                != (*kind == hiroute_domain::AgentKindV1::DeepseekHarness)
                            || matches!(settings, AgentModelSelectionV2::OpenCodeAdditional { .. })
                                != (*kind == hiroute_domain::AgentKindV1::OpenCode)
                        {
                            return Err(invalid());
                        }
                        SettingsModelAction::Additional(
                            pi_settings_content.clone(),
                            additional_dependencies.clone(),
                            super::AdditionalModelFileAction::Configure {
                                previous_operation: file.active_configuration.clone(),
                                provider_id: provider_id.clone(),
                                endpoint: endpoint.clone(),
                                models: models.clone(),
                            },
                        )
                    }
                    SettingsModelTargetFacts::Claude(claude) => {
                        let mut snapshot = claude_model_snapshot(
                            settings,
                            grant,
                            claude,
                            model
                                .claude()
                                .and_then(|facts| facts.presets.as_ref())
                                .ok_or_else(invalid)?,
                        )?;
                        snapshot.context_window_tokens = preview.claude_context_window;
                        SettingsModelAction::Claude(ClaudeModelFileAction::Configure {
                            previous_operation: file.active_configuration.clone(),
                            change: claude_native_change(
                                &snapshot,
                                claude,
                                &self.facts.context_id,
                                file.active_configuration.is_some(),
                            )?,
                            snapshot: Box::new(snapshot),
                            gateway_base_url: claude.gateway_base_url.clone(),
                            trusted_hiroute_executable: claude.trusted_hiroute_executable.clone(),
                        })
                    }
                };
                let material_action = match &preview.spec.access_token {
                    AgentAccessTokenIntentV1::Keep => AgentAccessGrantMaterialActionV1::Preserve,
                    AgentAccessTokenIntentV1::Regenerate => {
                        AgentAccessGrantMaterialActionV1::Regenerate
                    }
                    AgentAccessTokenIntentV1::Set { input_slot } => {
                        AgentAccessGrantMaterialActionV1::Set {
                            input_slot: input_slot.clone(),
                            fingerprint: file
                                .token_input_fingerprint
                                .clone()
                                .ok_or_else(invalid)?,
                        }
                    }
                };
                (
                    vec![
                        AgentAccessGrantMutationV1::ensure(
                            WorkspaceId::DEFAULT,
                            scope,
                            file.expected_grant_generation,
                        )?
                        .with_material_action(material_action)?,
                    ],
                    Some(action),
                    None,
                )
            }
            AgentFacetIntent::Restore { restore_point_ref } => {
                let file = self.model_file.as_ref().ok_or_else(invalid)?;
                self.facts.model.as_ref().ok_or_else(invalid)?;
                let (operation, reference) = file.restore.as_ref().ok_or_else(invalid)?;
                if *restore_point_ref != codex_model_restore_point_ref(operation)
                    || reference.generation() != file.expected_grant_generation
                {
                    return Err(invalid());
                }
                let action = match &file.target {
                    SettingsModelTargetFacts::Hermes { .. } => {
                        SettingsModelAction::Hermes(super::HermesModelFileAction::Restore {
                            original_operation: operation.clone(),
                        })
                    }
                    SettingsModelTargetFacts::Codex { .. } => {
                        SettingsModelAction::Codex(CodexModelFileAction::Restore {
                            original_operation: operation.clone(),
                            native_model: preview.spec.restore_native_model.clone(),
                        })
                    }
                    SettingsModelTargetFacts::Additional {
                        pi_settings_content,
                        additional_dependencies,
                        ..
                    } => SettingsModelAction::Additional(
                        pi_settings_content.clone(),
                        additional_dependencies.clone(),
                        super::AdditionalModelFileAction::Restore {
                            original_operation: operation.clone(),
                        },
                    ),
                    SettingsModelTargetFacts::Claude(_) => {
                        SettingsModelAction::Claude(ClaudeModelFileAction::Restore {
                            original_operation: operation.clone(),
                        })
                    }
                };
                (
                    vec![AgentAccessGrantMutationV1::revoke(
                        WorkspaceId::DEFAULT,
                        reference.connection_id(),
                        reference.generation(),
                    )?],
                    Some(action),
                    Some(reference.clone()),
                )
            }
            AgentFacetIntent::Keep => (Vec::new(), None, None),
        };
        let skill = self.skill_action(preview, preview.spec.skill_facet(), &self.skill_file)?;
        let additional_facet =
            if preview.spec.skill_facet() == hiroute_domain::AgentSettingsFacet::Collaboration {
                hiroute_domain::AgentSettingsFacet::ManagementSkill
            } else {
                hiroute_domain::AgentSettingsFacet::Collaboration
            };
        let additional = match &self.additional_skill_file {
            Some(file) => self.skill_action(preview, additional_facet, file)?,
            None if matches!(
                preview.spec.skill_intent_for(additional_facet),
                AgentFacetIntent::Keep
            ) =>
            {
                SettingsSkillAction::default()
            }
            None => return Err(invalid()),
        };
        // A permitted-plan selection can change without rewriting the shared Skill file: the
        // current root already carries the same template. Its desired collaboration state is
        // still durable in this confirmed settings operation, while Worker callability remains
        // outside this adapter until the directory/metadata owners publish a runtime binding.
        if model_action.is_none()
            && skill.intent.is_none()
            && !skill.record_change
            && additional.intent.is_none()
            && !additional.record_change
            && preview.spec.keeps_skills()
        {
            return Err(invalid());
        }
        let mut mutations = json!({
            "expected_revisions":self.expected_revisions,
            "skill_record_change":skill.record_change,
            "skill_reference_change":&skill.reference_change,
            "collaboration":self.collaboration_state,
        });
        if matches!(
            &self.model_file,
            Some(SettingsModelFileFacts {
                target: SettingsModelTargetFacts::Additional {
                    routing_adapter: Some(_),
                    ..
                },
                ..
            })
        ) {
            mutations["native_routing_adapter_file_change"] = json!(true);
        }
        if !matches!(
            preview.spec.skill_intent_for(additional_facet),
            AgentFacetIntent::Keep
        ) {
            mutations["additional_skill_record_change"] = json!(additional.record_change);
            mutations["additional_skill_reference_change"] = json!(&additional.reference_change);
        }
        confirmed.seal_effects_with_skills(
            self.subject.clone(),
            &mutations,
            skill.intent.is_some(),
            additional.intent.is_some(),
            model_mutations,
            |control| {
                let mut external = Vec::new();
                if self
                    .facts
                    .model
                    .as_ref()
                    .is_some_and(|model| model.common.login_item_removal_required)
                {
                    // The last managed connection's restore releases the login item this
                    // feature created in an older version. New connections never create one.
                    let declaration = confirmed.login_item().ok_or_else(invalid)?;
                    external.push(settings_login_item_intent(
                        control,
                        &self.facts.context_id,
                        declaration,
                    )?);
                }
                if let Some(action) = model_action {
                    let file = self.model_file.as_ref().ok_or_else(invalid)?;
                    let model = self.facts.model.as_ref().ok_or_else(invalid)?;
                    match action {
                        SettingsModelAction::Hermes(change) => {
                            let SettingsModelTargetFacts::Hermes { plugins, .. } = &file.target
                            else {
                                return Err(invalid());
                            };
                            if matches!(change, super::HermesModelFileAction::Configure { .. }) {
                                for facts in plugins {
                                    external.push(super::settings_native_plugin_file_intent(
                                        control,
                                        &self.facts.context_id,
                                        facts,
                                        &match &change {
                                            super::HermesModelFileAction::Configure {
                                                previous_operation,
                                                ..
                                            } => super::NativePluginFileAction::Configure {
                                                previous_operation: previous_operation.clone(),
                                                content: facts.content.into(),
                                            },
                                            super::HermesModelFileAction::Restore {
                                                original_operation,
                                            } => super::NativePluginFileAction::Restore {
                                                original_operation: original_operation.clone(),
                                            },
                                        },
                                    )?);
                                }
                            }
                            external.push(super::settings_hermes_model_file_intent(
                                control,
                                &self.facts.context_id,
                                file.expected_content.clone(),
                                file.before_fingerprint.clone(),
                                change.clone(),
                            )?);
                            if matches!(change, super::HermesModelFileAction::Restore { .. }) {
                                for facts in plugins {
                                    external.push(super::settings_native_plugin_file_intent(
                                        control,
                                        &self.facts.context_id,
                                        facts,
                                        &match &change {
                                            super::HermesModelFileAction::Configure {
                                                previous_operation,
                                                ..
                                            } => super::NativePluginFileAction::Configure {
                                                previous_operation: previous_operation.clone(),
                                                content: facts.content.into(),
                                            },
                                            super::HermesModelFileAction::Restore {
                                                original_operation,
                                            } => super::NativePluginFileAction::Restore {
                                                original_operation: original_operation.clone(),
                                            },
                                        },
                                    )?);
                                }
                            }
                        }
                        SettingsModelAction::Codex(change) => {
                            if let Some(catalog) =
                                model.codex().and_then(|facts| facts.model_catalog.as_ref())
                            {
                                // The immutable catalog artifact is staged before the managed
                                // configuration so the rendered pointer always has a target.
                                external.push(settings_codex_catalog_intent(
                                    control,
                                    &self.facts.context_id,
                                    catalog,
                                )?);
                            } else if matches!(
                                &preview.spec.model,
                                AgentFacetIntent::Configure { .. }
                            ) {
                                // Every Codex configuration publishes a filtered catalog.
                                return Err(invalid());
                            }
                            external.push(settings_codex_model_file_intent(
                                control,
                                &self.facts.context_id,
                                file.expected_content.clone(),
                                file.before_fingerprint.clone(),
                                change,
                            )?);
                        }
                        SettingsModelAction::Additional(
                            pi_settings_content,
                            additional_dependencies,
                            change,
                        ) => {
                            let SettingsModelTargetFacts::Additional {
                                routing_adapter, ..
                            } = &file.target
                            else {
                                return Err(invalid());
                            };
                            let resource_action =
                                routing_adapter.as_ref().map(|facts| match &change {
                                    super::AdditionalModelFileAction::Configure {
                                        previous_operation,
                                        ..
                                    } => super::NativePluginFileAction::Configure {
                                        previous_operation: previous_operation.clone(),
                                        content: facts.content.into(),
                                    },
                                    super::AdditionalModelFileAction::Restore {
                                        original_operation,
                                    } => super::NativePluginFileAction::Restore {
                                        original_operation: original_operation.clone(),
                                    },
                                });
                            if let (
                                Some(facts),
                                Some(action @ super::NativePluginFileAction::Configure { .. }),
                            ) = (routing_adapter, &resource_action)
                            {
                                external.push(super::settings_native_plugin_file_intent(
                                    control,
                                    &self.facts.context_id,
                                    facts,
                                    action,
                                )?);
                            }
                            external.push(super::settings_additional_model_file_intent(
                                control,
                                &self.facts.context_id,
                                file.expected_content.clone(),
                                file.before_fingerprint.clone(),
                                change,
                                pi_settings_content,
                                additional_dependencies,
                            )?);
                            if let (
                                Some(facts),
                                Some(action @ super::NativePluginFileAction::Restore { .. }),
                            ) = (routing_adapter, &resource_action)
                            {
                                external.push(super::settings_native_plugin_file_intent(
                                    control,
                                    &self.facts.context_id,
                                    facts,
                                    action,
                                )?);
                            }
                        }
                        SettingsModelAction::Claude(change) => {
                            external.push(settings_claude_model_file_intent(
                                control,
                                &self.facts.context_id,
                                file.expected_content.clone(),
                                file.before_fingerprint.clone(),
                                change,
                            )?);
                        }
                    }
                    let publication_digest = file.publication_digest.clone().ok_or_else(invalid)?;
                    external.push(settings_model_publication_intent(
                        control,
                        &self.facts.context_id,
                        publication_digest,
                        restore_grant,
                    )?);
                }
                if let Some(intent) = skill.intent {
                    external.push(intent.render(
                        control,
                        &self.facts.context_id,
                        preview.spec.skill_facet(),
                        &self.skill_file,
                    )?);
                }
                if let Some(intent) = additional.intent {
                    external.push(intent.render(
                        control,
                        &self.facts.context_id,
                        additional_facet,
                        self.additional_skill_file.as_ref().ok_or_else(invalid)?,
                    )?);
                }
                Ok(external)
            },
        )
    }

    fn skill_action(
        &self,
        preview: &AgentSettingsPreview,
        facet: hiroute_domain::AgentSettingsFacet,
        file: &SettingsSkillFileFacts,
    ) -> Result<SettingsSkillAction, OperationValidationError> {
        let invalid = || OperationValidationError::UnregisteredEffectPlan;
        match &preview.spec.skill_intent_for(facet) {
            AgentFacetIntent::Keep => Ok(SettingsSkillAction::default()),
            AgentFacetIntent::Configure { .. } => {
                let plan = plan_skill_install(
                    &file.root_ref,
                    &self.facts.context_id,
                    &file.template,
                    file.before.as_ref(),
                    file.observed_file.as_ref(),
                )
                .map_err(|_| invalid())?;
                let record_change = file.before.as_ref() != Some(&plan.next);
                let (intent, reference_change) = match plan.file_action {
                    SkillFileAction::Install => (Some(SkillIntent::Install(plan)), None),
                    SkillFileAction::Keep if record_change => (
                        None,
                        Some(settings_skill_reference_change(
                            &self.facts.context_id,
                            &file.target,
                            file.before.as_ref(),
                            &plan,
                            file.before_fingerprint.clone(),
                            SkillReferenceAction::Add,
                        )?),
                    ),
                    SkillFileAction::Keep => (None, None),
                    SkillFileAction::Remove | SkillFileAction::Conflict => {
                        return Err(invalid());
                    }
                };
                Ok(SettingsSkillAction {
                    intent,
                    reference_change,
                    record_change,
                })
            }
            AgentFacetIntent::Restore { restore_point_ref } => {
                let current = file.before.as_ref().ok_or_else(invalid)?;
                if self.facts.restore_points.get(restore_point_ref) != Some(&facet) {
                    return Err(invalid());
                }
                let plan =
                    plan_skill_remove(&self.facts.context_id, current, file.observed_file.as_ref())
                        .map_err(|_| invalid())?;
                let record_change = &plan.next != current;
                let (intent, reference_change) = match plan.file_action {
                    SkillFileAction::Remove => (Some(SkillIntent::Remove(plan)), None),
                    SkillFileAction::Keep if record_change => (
                        None,
                        Some(settings_skill_reference_change(
                            &self.facts.context_id,
                            &file.target,
                            Some(current),
                            &plan,
                            file.before_fingerprint.clone(),
                            SkillReferenceAction::Remove,
                        )?),
                    ),
                    SkillFileAction::Keep => (None, None),
                    SkillFileAction::Install | SkillFileAction::Conflict => {
                        return Err(invalid());
                    }
                };
                Ok(SettingsSkillAction {
                    intent,
                    reference_change,
                    record_change,
                })
            }
        }
    }
}

enum SettingsModelAction {
    Hermes(super::HermesModelFileAction),
    Additional(
        Option<CanonicalDigest>,
        Option<CanonicalDigest>,
        super::AdditionalModelFileAction,
    ),
    Codex(CodexModelFileAction),
    Claude(ClaudeModelFileAction),
}

#[derive(Default)]
struct SettingsSkillAction {
    intent: Option<SkillIntent>,
    reference_change: Option<SettingsSkillReferenceChange>,
    record_change: bool,
}

enum SkillIntent {
    Install(SkillOwnershipPlan),
    Remove(SkillOwnershipPlan),
}

impl SkillIntent {
    fn render(
        &self,
        control: &hiroute_domain::AgentConnectionControlIntentV1,
        context_id: &str,
        facet: hiroute_domain::AgentSettingsFacet,
        file: &SettingsSkillFileFacts,
    ) -> Result<hiroute_domain::ExternalEffectIntentV1, OperationValidationError> {
        let (plan, template) = match self {
            Self::Install(plan) => (plan, Some(&file.template)),
            Self::Remove(plan) => (plan, None),
        };
        settings_skill_file_intent_for_facet(
            control,
            context_id,
            facet,
            file.before.as_ref(),
            plan,
            template,
            file.before_fingerprint.clone(),
        )
    }
}

fn claude_model_snapshot(
    settings: &hiroute_domain::AgentModelSelectionV2,
    grant: &hiroute_domain::AgentModelGrantV2,
    facts: &SettingsClaudeModelFacts,
    native_presets: &hiroute_domain::AgentClaudePresetValuesV2,
) -> Result<ClaudeLaunchSnapshotIntent, OperationValidationError> {
    let invalid = || OperationValidationError::UnregisteredEffectPlan;
    let installation = &facts.installation;
    let profile = &installation.profile;
    if installation.agent_id != "agent_claude_default"
        || profile.profile_id != "claude-messages-v1"
        || profile.integration_profile_ref != "builtin/claude-messages/v1"
        || profile.client_protocol() != Some(AgentIngressProtocolV1::Messages)
        || !Path::new(&facts.trusted_hiroute_executable).is_absolute()
    {
        return Err(invalid());
    }
    facts
        .gateway_base_url
        .strip_suffix("/v1")
        .filter(|value| local_gateway_origin(value))
        .ok_or_else(invalid)?;
    let presets = grant
        .claude_preset_values(settings, native_presets)
        .map_err(|_| invalid())?;
    Ok(ClaudeLaunchSnapshotIntent {
        context_window_tokens: None,
        native_presets: native_presets.clone(),
        presets,
        executable: facts.executable.clone().ok_or_else(invalid)?,
        installation_digest: installation.observation_digest.clone(),
    })
}

fn claude_native_change(
    snapshot: &ClaudeLaunchSnapshotIntent,
    facts: &SettingsClaudeModelFacts,
    context_id: &str,
    reconfigure: bool,
) -> Result<hiroute_domain::AgentConfigChangeV1, OperationValidationError> {
    let invalid = || OperationValidationError::UnregisteredEffectPlan;
    let endpoint = facts
        .gateway_base_url
        .strip_suffix("/v1")
        .ok_or_else(invalid)?;
    let connection_id = format!("agent-connection/{context_id}");
    let mut desired = std::collections::BTreeMap::from([
        (
            "apiKeyHelper".to_owned(),
            Some(json!({
                "executable": facts.trusted_hiroute_executable,
                "argv": [hiroute_domain::HIDDEN_AGENT_GRANT_HELPER_VERB_V1, connection_id]
            })),
        ),
        ("hiroute.auth_environment".to_owned(), None),
        (
            "env.CLAUDE_CODE_GATEWAY_HINT_HEADERS".to_owned(),
            Some(json!("1")),
        ),
        ("env.ANTHROPIC_BASE_URL".to_owned(), Some(json!(endpoint))),
    ]);
    for (name, value) in [
        ("OPUS", &snapshot.presets.opus),
        ("SONNET", &snapshot.presets.sonnet),
        ("HAIKU", &snapshot.presets.haiku),
    ] {
        desired.insert(
            format!("env.ANTHROPIC_DEFAULT_{name}_MODEL"),
            value.as_ref().map(|value| json!(value)),
        );
    }
    for key in hiroute_domain::CLAUDE_CONTEXT_ENVIRONMENT {
        if snapshot.context_window_tokens.is_none() && !reconfigure {
            continue;
        }
        desired.insert(
            format!("env.{key}"),
            snapshot
                .context_window_tokens
                .map(|value| json!(value.to_string())),
        );
    }
    hiroute_domain::AgentConfigChangeV1::preview(&facts.user_document, desired)
        .map_err(|_| invalid())
}

fn local_gateway_origin(value: &str) -> bool {
    value
        .strip_prefix("http://127.0.0.1:")
        .or_else(|| value.strip_prefix("http://[::1]:"))
        .is_some_and(|port| {
            !port.is_empty()
                && port.bytes().all(|byte| byte.is_ascii_digit())
                && port.parse::<u16>().is_ok_and(|port| port != 0)
        })
}
