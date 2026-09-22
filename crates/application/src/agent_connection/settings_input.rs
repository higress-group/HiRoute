//! Settings facts transported from the real backend to the existing typed planner.
use super::*;
use hiroute_domain::{
    AgentAccessGrantMaterialActionV1, AgentAccessGrantMutationV1, AgentAccessGrantRefV1,
    AgentAccessGrantScopeV1, AgentAccessTokenIntentV1, AgentConnectionTransactionSubjectV1,
    AgentFacetIntent, AgentIngressProtocolV1, CanonicalDigest, OperationId,
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
    pub model_file: SettingsModelFileFacts,
    pub skill_file: SettingsSkillFileFacts,
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
    Codex {
        provider_id: String,
        endpoint: String,
    },
    Claude(SettingsClaudeModelFacts),
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
        let file = &self.model_file;
        let (model_mutations, model_action, restore_grant) = match &preview.spec.model {
            AgentFacetIntent::Configure { settings } => {
                let grant = preview.model_grant.as_ref().ok_or_else(invalid)?;
                let scope = AgentAccessGrantScopeV1::new(
                    format!("agent-connection/{}", self.facts.context_id),
                    grant.clone(),
                )?;
                let action = match &file.target {
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
                        model_catalog: self
                            .facts
                            .model_catalog
                            .as_ref()
                            .map(|catalog| catalog.content_digest.clone()),
                    }),
                    SettingsModelTargetFacts::Claude(claude) => {
                        let snapshot = claude_model_snapshot(
                            settings,
                            grant,
                            claude,
                            self.facts
                                .native_claude_presets
                                .as_ref()
                                .ok_or_else(invalid)?,
                        )?;
                        SettingsModelAction::Claude(ClaudeModelFileAction::Configure {
                            previous_operation: file.active_configuration.clone(),
                            change: claude_native_change(
                                &snapshot,
                                claude,
                                &self.facts.context_id,
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
                let (operation, reference) = file.restore.as_ref().ok_or_else(invalid)?;
                if *restore_point_ref != codex_model_restore_point_ref(operation)
                    || reference.generation() != file.expected_grant_generation
                {
                    return Err(invalid());
                }
                let action = match &file.target {
                    SettingsModelTargetFacts::Codex { .. } => {
                        SettingsModelAction::Codex(CodexModelFileAction::Restore {
                            original_operation: operation.clone(),
                            native_model: preview.spec.restore_native_model.clone(),
                        })
                    }
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
        let skill = self.skill_action(preview)?;
        // A permitted-plan selection can change without rewriting the shared Skill file: the
        // current root already carries the same template. Its desired collaboration state is
        // still durable in this confirmed settings operation, while Worker callability remains
        // outside this adapter until the directory/metadata owners publish a runtime binding.
        if model_action.is_none()
            && skill.intent.is_none()
            && !skill.record_change
            && matches!(preview.spec.collaboration, AgentFacetIntent::Keep)
        {
            return Err(invalid());
        }
        confirmed.seal_effects(
            self.subject.clone(),
            &json!({
                "expected_revisions":self.expected_revisions,
                "skill_record_change":skill.record_change,
                "skill_reference_change":&skill.reference_change,
                "collaboration":self.collaboration_state,
            }),
            skill.intent.is_some(),
            model_mutations,
            |control| {
                let mut external = Vec::new();
                if self.facts.login_item_removal_required {
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
                    match action {
                        SettingsModelAction::Codex(change) => {
                            if let Some(catalog) = self.facts.model_catalog.as_ref() {
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
                    external.push(intent.render(control, self)?);
                }
                Ok(external)
            },
        )
    }

    fn skill_action(
        &self,
        preview: &AgentSettingsPreview,
    ) -> Result<SettingsSkillAction, OperationValidationError> {
        let invalid = || OperationValidationError::UnregisteredEffectPlan;
        match &preview.spec.collaboration {
            AgentFacetIntent::Keep => Ok(SettingsSkillAction::default()),
            AgentFacetIntent::Configure { .. } => {
                let plan = plan_skill_install(
                    &self.skill_file.root_ref,
                    &self.facts.context_id,
                    &self.skill_file.template,
                    self.skill_file.before.as_ref(),
                    self.skill_file.observed_file.as_ref(),
                )
                .map_err(|_| invalid())?;
                let record_change = self.skill_file.before.as_ref() != Some(&plan.next);
                let (intent, reference_change) = match plan.file_action {
                    SkillFileAction::Install => (Some(SkillIntent::Install(plan)), None),
                    SkillFileAction::Keep if record_change => (
                        None,
                        Some(settings_skill_reference_change(
                            &self.facts.context_id,
                            &self.skill_file.target,
                            self.skill_file.before.as_ref(),
                            &plan,
                            self.skill_file.before_fingerprint.clone(),
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
                let current = self.skill_file.before.as_ref().ok_or_else(invalid)?;
                if self.facts.restore_points.get(restore_point_ref)
                    != Some(&hiroute_application_api::AgentSettingsFacet::Collaboration)
                {
                    return Err(invalid());
                }
                let plan = plan_skill_remove(
                    &self.facts.context_id,
                    current,
                    self.skill_file.observed_file.as_ref(),
                )
                .map_err(|_| invalid())?;
                let record_change = &plan.next != current;
                let (intent, reference_change) = match plan.file_action {
                    SkillFileAction::Remove => (Some(SkillIntent::Remove(plan)), None),
                    SkillFileAction::Keep if record_change => (
                        None,
                        Some(settings_skill_reference_change(
                            &self.facts.context_id,
                            &self.skill_file.target,
                            Some(current),
                            &plan,
                            self.skill_file.before_fingerprint.clone(),
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
        input: &AgentSettingsPlanningInput,
    ) -> Result<hiroute_domain::ExternalEffectIntentV1, OperationValidationError> {
        let (plan, template) = match self {
            Self::Install(plan) => (plan, Some(&input.skill_file.template)),
            Self::Remove(plan) => (plan, None),
        };
        settings_skill_file_intent(
            control,
            &input.facts.context_id,
            input.skill_file.before.as_ref(),
            plan,
            template,
            input.skill_file.before_fingerprint.clone(),
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
        || profile.client_protocol() != AgentIngressProtocolV1::Messages
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
