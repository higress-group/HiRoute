//! Fresh native/settings facts for the public V2 connect path.
use super::*;
use hiroute_application::agent_connection::{
    AgentSettingsFacts, AgentSettingsPlanningInput, CodexModelFileAction,
    CollaborationSkillTemplate, SettingsClaudeModelFacts, SettingsModelCatalogFacts,
    SettingsModelFileFacts, SettingsModelTargetFacts, SettingsSkillFileFacts, SkillFileAction,
    codex_model_restore_point_ref, decode_settings_login_item, is_settings_login_item,
    plan_skill_install, plan_skill_remove, settings_claude_model_file_for_operation,
    settings_codex_catalog_target, settings_codex_model_file_for_operation,
};
use hiroute_application::control::RoutingFactsPort;
use hiroute_application_api::{
    AgentAccessTokenIntentV1, AgentCollaborationTriggerModeV2, AgentFacetIntent,
    AgentSettingsFacet, AgentSettingsSpecV2,
};
use hiroute_cpa_bridge::BorrowedCodexAuthSpec;
use hiroute_domain::{
    AgentAccessGrantRefV1, AgentCapabilitySet, AgentFixedModelSelectionV2, AgentModelRouteV2,
    CandidateSelectionV1, CanonicalDigest, ComputeManagementRepositoryPort,
    NativeAgentArtifactPort, PublicationRepositoryPort, ReasoningSelectionV1, SecretStorePort,
    SupportedAgentInstallationV1, is_agent_access_grant_effect,
};
use hiroute_integrations::{
    CodexCatalogSummaryV1, CodexConfigurationScope, CodexDefaultPolicy, CodexNativeRestore,
    CodexSelectionTarget, DiscoveredAuthSource, codex_explicit_model,
    codex_explicit_reasoning_effort, restore_codex_native, sample_codex_catalog_plan,
};
use std::collections::{BTreeMap, BTreeSet};

#[path = "settings_codex_association.rs"]
mod codex_association;
#[path = "settings_facts/codex_models.rs"]
mod codex_models;
use codex_association::{
    connector_account_matches, endpoint_matches_target, mark_catalog_structure_proven,
};

const COLLABORATION_SKILL_EXPLICIT_REVISION: &str = "worker-instance-explicit-v1";
const COLLABORATION_SKILL_DEFAULT_REVISION: &str = "worker-instance-default-v1";
const COLLABORATION_SKILL_EXPLICIT_CONTENT: &str = "---\nname: hiroute-collaboration\ndescription: Delegate through HiRoute only when the user explicitly asks for delegation.\n---\n\n# HiRoute collaboration\n\nUse delegation only when the user explicitly requests it. Before delegating, run `hiroute worker plans` and select a published plan whose stated purpose matches the work; if none matches, say so instead of inventing one.\n\nStart executable work with `hiroute worker exec --plan <ID> --cwd <PATH> ...`. Permission policy defaults to `approve-all`; use a restricted policy only when the user asks for one and the selected harness supports it. Supply exactly one input source and keep the returned submission key, task ID, and run ID. Use the response's next actions to read progress, check status, wait, fetch results, continue, or recover an uncertain submission. Never replace an uncertain submission key automatically.\n\nDo not delegate coordination, plan discovery, status checks, waits, result summaries, or another delegation. A Worker must not recursively delegate. Never request or expose grants, capabilities, root identities, digests, revisions, credentials, or native session material.\n";
const COLLABORATION_SKILL_DEFAULT_CONTENT: &str = "---\nname: hiroute-collaboration\ndescription: Delegate suitable executable work through HiRoute by default.\n---\n\n# HiRoute collaboration\n\nDelegate suitable executable work by default. Before delegating, run `hiroute worker plans` and select a published plan whose stated purpose matches the work; if none matches, say so instead of inventing one.\n\nStart executable work with `hiroute worker exec --plan <ID> --cwd <PATH> ...`. Permission policy defaults to `approve-all`; use a restricted policy only when the user asks for one and the selected harness supports it. Supply exactly one input source and keep the returned submission key, task ID, and run ID. Use the response's next actions to read progress, check status, wait, fetch results, continue, or recover an uncertain submission. Never replace an uncertain submission key automatically.\n\nDo not delegate coordination, plan discovery, status checks, waits, result summaries, or another delegation. A Worker must not recursively delegate. Never request or expose grants, capabilities, root identities, digests, revisions, credentials, or native session material.\n";

struct CodexRestoreModelFacts {
    ids: Option<Vec<String>>,
    model: Option<String>,
    catalog: Option<CodexCatalogSummaryV1>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SettingsAgentClass {
    Codex,
    Claude,
}

impl SettingsAgentClass {
    pub(super) fn agent_id(self) -> &'static str {
        match self {
            Self::Codex => "agent_codex_default",
            Self::Claude => "agent_claude_default",
        }
    }

    fn context_segment(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }

    fn native_path(self, scanner: &FilesystemAgentScannerV1) -> std::path::PathBuf {
        match self {
            Self::Codex => scanner.codex_user_config_target(),
            Self::Claude => scanner.claude_user_settings_target(),
        }
    }

    pub(super) fn skill_root_ref(self) -> &'static str {
        match self {
            Self::Codex => "skill-root/agent_codex_default",
            Self::Claude => "skill-root/agent_claude_default",
        }
    }

    pub(super) fn skill_target(self) -> Result<String, hiroute_domain::OperationValidationError> {
        let (profile_id, integration_profile_ref) = match self {
            Self::Codex => (
                hiroute_integrations::CODEX_PROFILE_ID_V1,
                hiroute_integrations::CODEX_INTEGRATION_PROFILE_REF_V1,
            ),
            Self::Claude => (
                hiroute_integrations::CLAUDE_PROFILE_ID_V1,
                hiroute_integrations::CLAUDE_INTEGRATION_PROFILE_REF_V1,
            ),
        };
        let subject = AgentConnectionTransactionSubjectV1::from_registered_profile(
            self.agent_id(),
            profile_id,
            integration_profile_ref,
        )?;
        AgentConnectionEffectRoleV1::RoutingSkill.target_for(&subject)
    }
}

impl LocalControlAdapter {
    pub(super) fn settings_context_for_agent(&self, agent_id: &str) -> Option<String> {
        let class = match agent_id {
            "agent_codex_default" => SettingsAgentClass::Codex,
            "agent_claude_default" => SettingsAgentClass::Claude,
            _ => return None,
        };
        Some(self.settings_context(class))
    }

    pub(super) fn settings_agent_for_context(&self, context: &str) -> Option<SettingsAgentClass> {
        [SettingsAgentClass::Codex, SettingsAgentClass::Claude]
            .into_iter()
            .find(|class| context == self.settings_context(*class))
    }

    fn settings_context(&self, class: SettingsAgentClass) -> String {
        let path = class.native_path(&self.scanner);
        let digest = CanonicalDigest::of_bytes(path.as_os_str().as_encoded_bytes());
        format!(
            "agent-context/{}/{}",
            class.context_segment(),
            digest.as_str()
        )
    }

    pub(super) fn capture_settings_facts(
        &self,
        spec: &AgentSettingsSpecV2,
    ) -> Result<AgentSettingsPlanningInput, ControlReadError> {
        let class = self
            .settings_agent_for_context(&spec.context_id)
            .ok_or(ControlReadError::NotFound)?;
        let first = self.settings_snapshot(spec, class)?;
        let second = self.settings_snapshot(spec, class)?;
        if first.facts.dependency_digest != second.facts.dependency_digest {
            return Err(ControlReadError::SnapshotChanged);
        }
        Ok(second)
    }

    fn settings_snapshot(
        &self,
        spec: &AgentSettingsSpecV2,
        class: SettingsAgentClass,
    ) -> Result<AgentSettingsPlanningInput, ControlReadError> {
        let owned_claude = if class == SettingsAgentClass::Claude {
            if let Some(join) = self.configured_model_settings_join(&spec.context_id)? {
                let stores = self.stores_lock().map_err(super::map_port)?;
                let operation = stores
                    .control()
                    .load_operation(&join.operation_id)
                    .map_err(super::map_port)?
                    .ok_or(ControlReadError::Corrupt)?;
                let payload = settings_claude_model_file_for_operation(&operation, &join.intent)
                    .map_err(super::map_port)?;
                let hiroute_application::agent_connection::ClaudeModelFileAction::Configure {
                    change,
                    ..
                } = payload.change
                else {
                    return Err(ControlReadError::Corrupt);
                };
                Some(
                    self.scanner
                        .claude_installation_for_applied_user_change(&change)
                        .map_err(|_| ControlReadError::SnapshotChanged)?,
                )
            } else {
                None
            }
        } else {
            None
        };
        let discovery = if class == SettingsAgentClass::Claude {
            self.scanner.claude_settings_discovery(matches!(
                &spec.collaboration,
                AgentFacetIntent::Configure { .. }
            ))
        } else {
            self.scanner.codex_settings_discovery(matches!(
                &spec.collaboration,
                AgentFacetIntent::Configure { .. }
            ))
        };
        let mut installation = match &owned_claude {
            Some((installation, _)) => installation.clone(),
            None => match &discovery.outcome {
                AgentDiscoveryOutcomeV1::Supported { installation }
                    if installation.agent_id == class.agent_id() =>
                {
                    (**installation).clone()
                }
                _ => return Err(ControlReadError::NotFound),
            },
        };
        #[cfg(unix)]
        if class == SettingsAgentClass::Claude
            && matches!(&spec.collaboration, AgentFacetIntent::Configure { .. })
            && let Some((_, executable)) = &owned_claude
        {
            self.scanner.attach_claude_collaboration_evidence(
                std::path::Path::new(executable),
                &mut installation,
            );
        }
        if class == SettingsAgentClass::Claude
            && matches!(spec.model, AgentFacetIntent::Configure { .. })
            && self
                .scanner
                .claude_native_routing_conflict()
                .map_err(|_| ControlReadError::Corrupt)?
            && let Some(evidence) = installation
                .capability_evidence
                .iter_mut()
                .find(|evidence| {
                    evidence.capability == hiroute_domain::AgentCapability::EffectiveConfiguration
                })
        {
            evidence.state = hiroute_domain::CapabilityState::Unknown;
            evidence.reason = Some(hiroute_domain::CapabilityReason::HigherPrecedenceConflict);
        }
        let claude_executable = owned_claude.map(|(_, executable)| executable).or_else(|| {
            discovery
                .managed_launch
                .as_ref()
                .filter(|preflight| preflight.is_launchable())
                .map(|preflight| preflight.executable.clone())
        });
        let subject = AgentConnectionTransactionSubjectV1::from_registered_profile(
            installation.agent_id.clone(),
            installation.profile.profile_id.clone(),
            installation.profile.integration_profile_ref.clone(),
        )
        .map_err(|_| ControlReadError::Corrupt)?;
        let target = AgentConnectionEffectRoleV1::ManagedConfiguration
            .settings_target_for(&subject)
            .map_err(|_| ControlReadError::Corrupt)?;
        let before_fingerprint = self
            .artifacts
            .current_external_fingerprint(&target)
            .map_err(super::map_port)?;
        let bytes = self
            .artifacts
            .read_native_target(&target)
            .map_err(super::map_port)?;
        let expected_content =
            CanonicalDigest::of_bytes(bytes.as_deref().map_or(&[], |bytes| bytes.as_slice()));
        if before_fingerprint
            != self
                .artifacts
                .current_external_fingerprint(&target)
                .map_err(super::map_port)?
        {
            return Err(ControlReadError::SnapshotChanged);
        }
        drop(bytes);

        let skill_target = AgentConnectionEffectRoleV1::RoutingSkill
            .target_for(&subject)
            .map_err(|_| ControlReadError::Corrupt)?;
        let skill_before_fingerprint = self
            .artifacts
            .current_external_fingerprint(&skill_target)
            .map_err(super::map_port)?;
        let skill_bytes = self
            .artifacts
            .read_native_target(&skill_target)
            .map_err(super::map_port)?;
        let observed_skill_file = skill_bytes
            .as_deref()
            .map(|bytes| CanonicalDigest::of_bytes(bytes.as_slice()));
        if skill_before_fingerprint
            != self
                .artifacts
                .current_external_fingerprint(&skill_target)
                .map_err(super::map_port)?
        {
            return Err(ControlReadError::SnapshotChanged);
        }
        drop(skill_bytes);

        let runtime = self
            .managed_agent_runtime
            .lock()
            .map_err(|_| ControlReadError::Unavailable)?
            .clone()
            .ok_or(ControlReadError::Unavailable)?;
        let stores = self.stores_lock().map_err(super::map_port)?;
        let workspace = WorkspaceId::default();
        let expected_revisions = stores
            .control()
            .current_revisions(&workspace)
            .map_err(super::map_port)?;
        let publication = stores
            .control()
            .active_publication(&workspace)
            .map_err(super::map_port)?;
        if publication.is_none() && !matches!(spec.model, AgentFacetIntent::Keep) {
            // Model grants and their file effects are always coupled to an exact publication.
            // Only the independent collaboration facet is valid before the first publication.
            return Err(ControlReadError::NotFound);
        }
        let active = publication
            .as_ref()
            .map(|record| record.verify().map_err(|_| ControlReadError::Corrupt))
            .transpose()?;
        let current_grant = stores
            .secrets()
            .inspect_agent_access_grant(
                WorkspaceId::DEFAULT,
                &format!("agent-connection/{}", spec.context_id),
            )
            .map_err(super::map_port)?;
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
                        .map_err(super::map_port)?,
                )
            }
            _ => return Err(ControlReadError::Denied),
        };
        // New settings saves do not manage startup. Keep the preview field for V2 clients;
        // only an older, journal-owned login item may require cleanup on final restore.
        let other_context = match class {
            SettingsAgentClass::Codex => SettingsAgentClass::Claude,
            SettingsAgentClass::Claude => SettingsAgentClass::Codex,
        };
        let other_grant = stores
            .secrets()
            .inspect_agent_access_grant(
                WorkspaceId::DEFAULT,
                &format!("agent-connection/{}", self.settings_context(other_context)),
            )
            .map_err(super::map_port)?;
        let login_item_required = false;
        // Releasing the last managed connection also releases the login item this feature
        // owns. Ownership is proven only by journaled evidence — the newest login-item
        // declaration across succeeded settings operations: an establishment this feature
        // created owns the item, a removal releases it, and a pre-existing user item
        // (established without creation) is never owned and never removed.
        let login_item_removal_required = !runtime.resident_service_ready
            && current_grant.is_some()
            && other_grant.is_none()
            && matches!(spec.model, AgentFacetIntent::Restore { .. })
            && login_item_owned_by_this_feature(&stores, &workspace)?;
        let expected_grant_generation = stores
            .secrets()
            .agent_access_grant_generation(
                WorkspaceId::DEFAULT,
                &format!("agent-connection/{}", spec.context_id),
            )
            .map_err(super::map_port)?;
        let skill_before = stores
            .control()
            .skill_installation(&workspace, class.skill_root_ref())
            .map_err(super::map_port)?;

        let mut restore_points = BTreeMap::new();
        let mut active_configuration = None;
        let mut active_model_selection = None;
        let mut active_protected_native_ids = Vec::new();
        let mut selected_restore = None;
        let mut selected_collaboration_restore = None;
        for original in stores
            .control()
            .succeeded_operations_for_kind(&workspace, "ApplyAgentConnectionChange")
            .map_err(super::map_port)?
        {
            if original.plan.spec().command_id != "agents.settings.apply" {
                continue;
            }
            let original_spec: AgentSettingsSpecV2 =
                match serde_json::from_value(original.plan.spec().desired_state.clone()) {
                    Ok(value) => value,
                    Err(_) => return Err(ControlReadError::Corrupt),
                };
            if original_spec.context_id != spec.context_id {
                continue;
            }
            if selected_collaboration_restore.is_none()
                && matches!(
                    original_spec.collaboration,
                    AgentFacetIntent::Configure { .. }
                )
                && skill_before
                    .as_ref()
                    .is_some_and(|record| record.contexts.contains(&spec.context_id))
            {
                selected_collaboration_restore = Some(original.operation_id.clone());
            }
            let model = match class {
                SettingsAgentClass::Codex => original
                    .plan
                    .external()
                    .iter()
                    .find(|intent| super::native_model::is_settings_codex_model(intent))
                    .map(|intent| {
                        settings_codex_model_file_for_operation(&original, intent)
                            .map(|payload| matches!(payload.change, CodexModelFileAction::Configure { .. }))
                    }),
                SettingsAgentClass::Claude => original
                    .plan
                    .external()
                    .iter()
                    .find(|intent| super::native_claude_model::is_settings_claude_model(intent))
                    .map(|intent| {
                        settings_claude_model_file_for_operation(&original, intent).map(|payload| {
                            matches!(
                                payload.change,
                                hiroute_application::agent_connection::ClaudeModelFileAction::Configure { .. }
                            )
                        })
                    }),
            };
            let Some(is_configure) = model else {
                continue;
            };
            if !is_configure.map_err(super::map_port)? {
                continue;
            }
            let [mutation] = original.plan.agent_access_grants() else {
                return Err(ControlReadError::Corrupt);
            };
            let effect = original
                .step(hiroute_domain::OperationStepKind::ApplySecrets)
                .effects
                .iter()
                .find(|effect| is_agent_access_grant_effect(effect))
                .ok_or(ControlReadError::Corrupt)?;
            let reference = AgentAccessGrantRefV1::from_ensure_effect(effect, mutation)
                .map_err(|_| ControlReadError::Corrupt)?;
            if current_grant.as_ref() != Some(&reference) {
                continue;
            }
            // Successful operations are projected newest first. An identical settings apply can
            // legitimately reuse the active grant, so keep the newest operation as the file
            // lineage owner while retaining older restore points for that same grant.
            if active_configuration.is_none() {
                active_configuration = Some(original.operation_id.clone());
                active_protected_native_ids = original_spec.protected_native_model_ids.clone();
                active_model_selection = match &original_spec.model {
                    AgentFacetIntent::Configure { settings } => Some(settings.clone()),
                    _ => return Err(ControlReadError::Corrupt),
                };
            }
            let restore_ref = codex_model_restore_point_ref(&original.operation_id);
            restore_points.insert(restore_ref.clone(), AgentSettingsFacet::Model);
            if matches!(&spec.model, AgentFacetIntent::Restore { restore_point_ref } if restore_point_ref == &restore_ref)
            {
                selected_restore = Some((original.operation_id.clone(), reference));
            }
        }
        if current_grant.is_some() != active_configuration.is_some() {
            return Err(ControlReadError::Corrupt);
        }
        if let Some(operation) = selected_collaboration_restore {
            restore_points.insert(
                codex_model_restore_point_ref(&operation),
                AgentSettingsFacet::Collaboration,
            );
        }
        drop(stores);
        let (restore_native_model_ids, restored_native_model) = if class
            == SettingsAgentClass::Codex
            && matches!(&spec.model, AgentFacetIntent::Restore { .. })
        {
            match selected_restore.as_ref() {
                Some((original, _)) => {
                    let facts = self.codex_restore_model_facts(original, &target)?;
                    (facts.ids, facts.model)
                }
                None => (Some(Vec::new()), None),
            }
        } else {
            (None, None)
        };
        let trigger_mode = match &spec.collaboration {
            AgentFacetIntent::Configure { settings } => settings.trigger_mode,
            AgentFacetIntent::Keep | AgentFacetIntent::Restore { .. } => {
                AgentCollaborationTriggerModeV2::Explicit
            }
        };
        let (template_revision, template_content) = match trigger_mode {
            AgentCollaborationTriggerModeV2::Explicit => (
                COLLABORATION_SKILL_EXPLICIT_REVISION,
                COLLABORATION_SKILL_EXPLICIT_CONTENT,
            ),
            AgentCollaborationTriggerModeV2::DelegateByDefault => (
                COLLABORATION_SKILL_DEFAULT_REVISION,
                COLLABORATION_SKILL_DEFAULT_CONTENT,
            ),
        };
        let template = CollaborationSkillTemplate::bundled(template_revision, template_content)
            .map_err(|_| ControlReadError::Corrupt)?;
        let collaboration_file_conflict = match &spec.collaboration {
            AgentFacetIntent::Configure { .. } => plan_skill_install(
                class.skill_root_ref(),
                &spec.context_id,
                &template,
                skill_before.as_ref(),
                observed_skill_file.as_ref(),
            )
            .is_err(),
            AgentFacetIntent::Restore { .. } => skill_before.as_ref().is_some_and(|current| {
                plan_skill_remove(&spec.context_id, current, observed_skill_file.as_ref())
                    .map_or(true, |plan| plan.file_action == SkillFileAction::Conflict)
            }),
            AgentFacetIntent::Keep => false,
        };
        let mut fixed_candidate_facts = match &spec.model {
            AgentFacetIntent::Configure { settings }
                if class == SettingsAgentClass::Codex || !settings.fixed_models().is_empty() =>
            {
                match self.routing_compilation_snapshot(&workspace) {
                    Ok(snapshot) => snapshot.facts.candidates,
                    // An installation may have a native catalog before the user has imported any
                    // HiRoute source. Preview must surface the source blocker, not become 404.
                    Err(ControlReadError::NotFound) => Vec::new(),
                    Err(error) => return Err(error),
                }
            }
            _ => Vec::new(),
        };
        let preserve_native_models = matches!(
            &spec.model,
            AgentFacetIntent::Configure {
                settings: hiroute_domain::AgentModelSelectionV2::CodexDefault {
                    native_model_mode: hiroute_domain::CodexNativeModelModeV2::PreserveAvailable,
                    ..
                }
            }
        );
        let codex_catalog_summary = if class == SettingsAgentClass::Codex
            && preserve_native_models
            && let Some(active) = active_configuration.as_ref()
        {
            self.codex_restore_model_facts(active, &target)?.catalog
        } else if class == SettingsAgentClass::Codex {
            self.scanner.codex_catalog_summary().ok()
        } else {
            None
        };
        let native_default_model = match class {
            SettingsAgentClass::Codex => codex_catalog_summary
                .as_ref()
                .map(|catalog| catalog.native_default_model.clone()),
            SettingsAgentClass::Claude => self
                .scanner
                .claude_explicit_model_selection()
                .map_err(|_| ControlReadError::Corrupt)?,
        };
        let native_claude_presets = if matches!(class, SettingsAgentClass::Claude)
            && matches!(spec.model, AgentFacetIntent::Configure { .. })
        {
            let value = |id: &str| -> Result<Option<String>, ControlReadError> {
                let field = installation
                    .profile
                    .field(id)
                    .ok_or(ControlReadError::Corrupt)?;
                installation
                    .effective_config
                    .get(&field.path)
                    .map(|field| {
                        field
                            .value
                            .as_str()
                            .filter(|value| hiroute_domain::valid_client_model_name(value))
                            .map(str::to_owned)
                            .ok_or(ControlReadError::Corrupt)
                    })
                    .transpose()
            };
            if let Some(active) = &active_configuration {
                let stores = self.stores_lock().map_err(super::map_port)?;
                let operation = stores
                    .control()
                    .load_operation(active)
                    .map_err(super::map_port)?
                    .ok_or(ControlReadError::Corrupt)?;
                let intent = operation
                    .plan
                    .external()
                    .iter()
                    .find(|intent| super::native_claude_model::is_settings_claude_model(intent))
                    .ok_or(ControlReadError::Corrupt)?;
                let payload = settings_claude_model_file_for_operation(&operation, intent)
                    .map_err(super::map_port)?;
                match payload.change {
                    hiroute_application::agent_connection::ClaudeModelFileAction::Configure {
                        snapshot,
                        ..
                    } => Some(snapshot.native_presets),
                    _ => return Err(ControlReadError::Corrupt),
                }
            } else {
                Some(hiroute_domain::AgentClaudePresetValuesV2 {
                    opus: value("default_opus_model")?,
                    sonnet: value("default_sonnet_model")?,
                    haiku: value("default_haiku_model")?,
                })
            }
        } else {
            None
        };
        let collaboration_state = self.settings_collaboration_state(spec)?;
        let original_native_ids = if class == SettingsAgentClass::Codex
            && preserve_native_models
            && matches!(&spec.model, AgentFacetIntent::Configure { .. })
        {
            codex_catalog_summary.as_ref().map(|catalog| {
                catalog
                    .models
                    .iter()
                    .map(|model| model.client_model_id.clone())
                    .collect::<Vec<_>>()
            })
        } else {
            None
        };
        let mut preserved_codex_bindings = BTreeMap::new();
        let (preserved_codex_models, proven_native_model_ids) =
            match (&spec.model, codex_catalog_summary.as_ref()) {
                (
                    AgentFacetIntent::Configure {
                        settings: hiroute_domain::AgentModelSelectionV2::CodexDefault { .. },
                    },
                    Some(catalog),
                ) if preserve_native_models && active_protected_native_ids.is_empty() => self
                    .preserved_codex_model_selections(catalog, &mut fixed_candidate_facts)
                    .unwrap_or_default(),
                (AgentFacetIntent::Configure { .. }, _)
                    if preserve_native_models && !active_protected_native_ids.is_empty() =>
                {
                    let previous = active_model_selection
                        .as_ref()
                        .ok_or(ControlReadError::Corrupt)?;
                    let grant = current_grant.as_ref().ok_or(ControlReadError::Corrupt)?;
                    let mut preserved = Vec::new();
                    for name in &active_protected_native_ids {
                        let selected = previous
                            .fixed_models()
                            .iter()
                            .find(|fixed| &fixed.client_model_id == name)
                            .ok_or(ControlReadError::Corrupt)?;
                        let AgentModelRouteV2::Fixed { candidate, binding } = grant
                            .scope()
                            .model_grant()
                            .routes
                            .get(name)
                            .ok_or(ControlReadError::Corrupt)?
                        else {
                            return Err(ControlReadError::Corrupt);
                        };
                        if candidate != &selected.candidate
                            || binding.binding_id != candidate.binding_id
                            || binding.upstream_model_id != name.as_str()
                        {
                            return Err(ControlReadError::Corrupt);
                        }
                        preserved_codex_bindings.insert(name.clone(), binding.as_ref().clone());
                        preserved.push(selected.clone());
                    }
                    (preserved, active_protected_native_ids.clone())
                }
                _ => (Vec::new(), Vec::new()),
            };
        let required_native_model_ids = preserve_native_models.then(|| {
            if active_protected_native_ids.is_empty() {
                proven_native_model_ids.clone()
            } else {
                active_protected_native_ids.clone()
            }
        });
        let unproven_native_model_ids = original_native_ids
            .unwrap_or_default()
            .into_iter()
            .filter(|name| !proven_native_model_ids.contains(name))
            .collect::<Vec<_>>();
        let native_default_must_be_original = preserve_native_models;
        let mut model_catalog =
            if let (SettingsAgentClass::Codex, AgentFacetIntent::Configure { settings }) =
                (class, &spec.model)
            {
                let baseline = {
                    let stores = self.stores_lock().map_err(super::map_port)?;
                    super::native_model::codex_catalog_baseline(
                        stores.control(),
                        &self.artifacts,
                        active_configuration.as_ref(),
                        &target,
                    )
                    .map_err(super::map_port)?
                };
                let mut complete = settings.clone();
                if let hiroute_domain::AgentModelSelectionV2::CodexDefault {
                    fixed_models, ..
                } = &mut complete
                {
                    for preserved in &preserved_codex_models {
                        if !fixed_models
                            .iter()
                            .any(|model| model.client_model_id == preserved.client_model_id)
                        {
                            fixed_models.push(preserved.clone());
                        }
                    }
                }
                self.codex_catalog_facts(&complete, active.as_ref(), &baseline, &mut installation)
            } else {
                None
            };
        if let Some(catalog) = model_catalog.as_mut() {
            let catalog_target = settings_codex_catalog_target(&subject, &spec.context_id, catalog)
                .map_err(|_| ControlReadError::Corrupt)?;
            let observed = self
                .artifacts
                .current_external_fingerprint(&catalog_target)
                .map_err(super::map_port)?;
            if observed
                != self
                    .artifacts
                    .current_external_fingerprint(&catalog_target)
                    .map_err(super::map_port)?
            {
                return Err(ControlReadError::SnapshotChanged);
            }
            catalog.before_fingerprint = observed;
        }
        let semantic_evidence: Vec<_> = installation
            .capability_evidence
            .iter()
            .map(|item| {
                (
                    item.capability,
                    item.state,
                    &item.adapter_contract,
                    &item.dependency_digest,
                    item.reason,
                )
            })
            .collect();
        let releases_borrowed_skill_reference =
            matches!(&spec.collaboration, AgentFacetIntent::Restore { .. })
                && skill_before.as_ref().is_some_and(|record| {
                    record.file_ownership
                        == hiroute_domain::CollaborationSkillFileOwnership::BorrowedIdentical
                        && record.contexts.contains(&spec.context_id)
                });
        let dependency_digest = CanonicalDigest::of(&serde_json::json!({
            "context":spec.context_id,"agent":class.context_segment(),"observation":installation.observation_digest,
            "claude_executable": if class == SettingsAgentClass::Claude { claude_executable.as_deref() } else { None },
            "capabilities":semantic_evidence,"content":expected_content,"fingerprint":before_fingerprint,
            "skill_content":if releases_borrowed_skill_reference { None } else { observed_skill_file.as_ref() },
            "skill_fingerprint":if releases_borrowed_skill_reference { None } else { skill_before_fingerprint.as_ref() },
            "skill_record":skill_before,"skill_template":template.digest,"revisions":expected_revisions,
            "publication":publication.as_ref().map(|record| &record.digest),"grant":current_grant,"grant_generation":expected_grant_generation,
            "token_input_fingerprint":token_input_fingerprint,
            "restores":restore_points,"endpoint":runtime.gateway_base_url,
            "worker_channel":"local_trust_v1",
            "collaboration_state":collaboration_state,
            "fixed_candidates":fixed_candidate_facts,"native_default_model":native_default_model,
            "preserved_codex_models":preserved_codex_models,
            "preserved_codex_bindings":preserved_codex_bindings,
            "required_native_model_ids":required_native_model_ids,
            "require_native_model_routes":preserve_native_models,
            "unproven_native_model_ids":unproven_native_model_ids,
            "native_default_must_be_original":native_default_must_be_original,
            "native_claude_presets":native_claude_presets,
            "restore_native_model_ids":restore_native_model_ids,
            "restored_native_model":restored_native_model,
            "collaboration_file_conflict":collaboration_file_conflict,
            "model_catalog":&model_catalog,
            "resident_service":login_item_required,
            "resident_service_removal":login_item_removal_required,
        }))
        .map_err(|_| ControlReadError::Corrupt)?;
        let capabilities =
            AgentCapabilitySet::new(installation.capability_evidence.iter().cloned().map(
                |mut item| {
                    // Carry only evidence validated against this exact observation into the larger
                    // settings dependency set; stale source evidence stays stale and cannot authorize.
                    if item.dependency_digest == installation.observation_digest {
                        item.dependency_digest = dependency_digest.clone();
                    }
                    item
                },
            ))
            .map_err(|_| ControlReadError::Corrupt)?;
        let model_target = match class {
            SettingsAgentClass::Codex => SettingsModelTargetFacts::Codex {
                provider_id: "hiroute".into(),
                endpoint: runtime.gateway_base_url.clone(),
            },
            SettingsAgentClass::Claude => {
                SettingsModelTargetFacts::Claude(SettingsClaudeModelFacts {
                    installation: installation.clone(),
                    executable: claude_executable,
                    gateway_base_url: runtime.gateway_base_url.clone(),
                    trusted_hiroute_executable: runtime.trusted_hiroute_executable.clone(),
                    user_document: self
                        .scanner
                        .claude_user_config_document()
                        .map_err(|_| ControlReadError::Corrupt)?,
                })
            }
        };
        Ok(AgentSettingsPlanningInput {
            collaboration_state,
            expected_revisions,
            subject,
            model_file: SettingsModelFileFacts {
                expected_content,
                before_fingerprint,
                publication_digest: publication.map(|record| record.digest),
                expected_grant_generation,
                token_input_fingerprint,
                active_configuration,
                restore: selected_restore,
                target: model_target,
            },
            skill_file: SettingsSkillFileFacts {
                root_ref: class.skill_root_ref().into(),
                target: skill_target,
                before: skill_before,
                observed_file: observed_skill_file,
                before_fingerprint: skill_before_fingerprint,
                template,
            },
            facts: AgentSettingsFacts {
                context_id: spec.context_id.clone(),
                dependency_digest,
                capabilities,
                ingress: installation.profile.client_protocol(),
                available_surfaces: self.scanner.available_model_surfaces(class.agent_id()),
                model_publication: active,
                model_catalog,
                login_item_required,
                login_item_removal_required,
                fixed_candidate_facts,
                preserved_codex_models,
                preserved_codex_bindings,
                required_native_model_ids,
                require_native_model_routes: preserve_native_models,
                unproven_native_model_ids,
                native_default_must_be_original,
                native_default_model,
                native_claude_presets,
                restore_native_model_ids,
                restored_native_model,
                collaboration_file_conflict,
                restore_points,
            },
        })
    }
}

/// The newest journaled login-item declaration decides whether this feature owns the login
/// item: an establishment this feature created owns it, a removal releases it, and a
/// pre-existing user item is never owned. Succeeded operations are visited newest first.
fn login_item_owned_by_this_feature(
    stores: &hiroute_local_storage::LocalStorageSet,
    workspace: &WorkspaceId,
) -> Result<bool, ControlReadError> {
    for original in stores
        .control()
        .succeeded_operations_for_kinds(
            workspace,
            &["ApplyAgentConnectionChange", "ApplyAgentConnectionRestore"],
        )
        .map_err(super::map_port)?
    {
        if let Some(intent) = original
            .plan
            .external()
            .iter()
            .find(|intent| is_settings_login_item(intent))
        {
            let payload = decode_settings_login_item(intent).map_err(super::map_port)?;
            return Ok(payload.created());
        }
    }
    Ok(false)
}
