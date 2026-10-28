//! Fresh native/settings facts for the public V2 connect path.
use super::*;
use hiroute_application::agent_connection::{
    AgentSettingsFacts, AgentSettingsPlanningInput, CodexModelFileAction,
    CollaborationSkillTemplate, SettingsAdditionalModelFacts, SettingsClaudeModelFacts,
    SettingsClaudeNativeFacts, SettingsCodexModelFacts, SettingsModelCatalogFacts,
    SettingsModelCommonFacts, SettingsModelFacts, SettingsModelFileFacts, SettingsModelNativeFacts,
    SettingsModelTargetFacts, SettingsSkillFileFacts, SkillFileAction,
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
use hiroute_diagnostics::agent_read::{
    AgentReadCategory as Category, AgentReadStage as Stage, Timer,
};
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

mod additional_model;
#[path = "settings_codex_association.rs"]
mod codex_association;
#[path = "settings_facts/codex_models.rs"]
mod codex_models;
#[cfg(test)]
pub(crate) mod diagnostics;
mod hermes;
mod hermes_model;
mod qoder;
pub(super) mod skills;
use codex_association::{
    connector_account_matches, endpoint_matches_target, mark_catalog_structure_proven,
};

const COLLABORATION_SKILL_EXPLICIT_REVISION: &str = "worker-instance-explicit-v2";
const COLLABORATION_SKILL_DEFAULT_REVISION: &str = "worker-instance-default-v3";
const COLLABORATION_SKILL_EXPLICIT_CONTENT: &str = "---\nname: hiroute-collaboration\ndescription: Delegate through HiRoute only when the user explicitly asks for delegation.\n---\n\n# HiRoute collaboration\n\nUse delegation only when the user explicitly requests it. Before delegating, run `hiroute worker plans` and select a published plan whose stated purpose matches the work; if none matches, say so instead of inventing one.\n\nStart executable work with `hiroute worker exec --plan <ID> --cwd <PATH> ...`. Permission policy defaults to `approve-all`; use a restricted policy only when the user asks for one and the selected harness supports it. For each new exec or continue, choose a unique submission key and put its literal value in `--submission-key` before invoking the CLI; do not generate it dynamically inside the command. Reuse that key only when retrying the identical request. Use a short recognizable `--title` for a new task. Supply exactly one input source and keep the submission key, task ID, and run ID in the conversation. The CLI keeps no local submission receipt. Use the response's next actions to read progress, check status, wait, fetch results, continue, or recover an uncertain submission. If a response is lost, query `hiroute worker status --submission-key <ORIGINAL_KEY> --operation <start|continue>` first. If the key is lost, use `hiroute worker list --title <TITLE>`, follow all pages, and confirm the workspace, time and task details before using an existing task ID. Titles may repeat and disappear after seven days or content deletion. An empty list or unclear match does not prove that submission failed: stop and ask the user instead of automatically submitting again or replacing the key.\n\nDo not delegate coordination, plan discovery, status checks, waits, result summaries, or another delegation. A Worker must not recursively delegate. Never request or expose grants, capabilities, root identities, digests, revisions, credentials, or native session material.\n";
const COLLABORATION_SKILL_DEFAULT_CONTENT: &str = "---\nname: hiroute-collaboration\ndescription: Delegate suitable executable work through HiRoute by default.\n---\n\n# HiRoute collaboration\n\nDelegate suitable executable work by default. Respect answer-only requests: do not start or delegate work when the user asks only for an answer. Before delegating, run `hiroute worker plans` and select a published plan whose stated purpose matches the work; if none matches, say so instead of inventing one.\n\nStart executable work with `hiroute worker exec --plan <ID> --cwd <PATH> ...`. Permission policy defaults to `approve-all`; use a restricted policy only when the user asks for one and the selected harness supports it. For each new exec or continue, choose a unique submission key and put its literal value in `--submission-key` before invoking the CLI; do not generate it dynamically inside the command. Reuse that key only when retrying the identical request. Use a short recognizable `--title` for a new task. Supply exactly one input source and keep the submission key, task ID, and run ID in the conversation. The CLI keeps no local submission receipt. Use the response's next actions to read progress, check status, wait, fetch results, continue, or recover an uncertain submission. If a response is lost, query `hiroute worker status --submission-key <ORIGINAL_KEY> --operation <start|continue>` first. If the key is lost, use `hiroute worker list --title <TITLE>`, follow all pages, and confirm the workspace, time and task details before using an existing task ID. Titles may repeat and disappear after seven days or content deletion. An empty list or unclear match does not prove that submission failed: stop and ask the user instead of automatically submitting again or replacing the key.\n\nDo not delegate coordination, plan discovery, status checks, waits, result summaries, or another delegation. A Worker must not recursively delegate. Never request or expose grants, capabilities, root identities, digests, revisions, credentials, or native session material.\n";

struct CodexRestoreModelFacts {
    ids: Option<Vec<String>>,
    model: Option<String>,
    catalog: Option<CodexCatalogSummaryV1>,
}

mod class;
pub(super) use class::SettingsAgentClass;

impl LocalControlAdapter {
    pub(super) fn settings_context_for_agent(&self, agent_id: &str) -> Option<String> {
        let class = match agent_id {
            "agent_codex_default" => SettingsAgentClass::Codex,
            "agent_claude_default" => SettingsAgentClass::Claude,
            "agent_qoder_default" => SettingsAgentClass::Qoder,
            "agent_pi_default" => SettingsAgentClass::Pi,
            "agent_dsh_default" => SettingsAgentClass::Dsh,
            "agent_dsh_desktop" => SettingsAgentClass::DshDesktop,
            "agent_opencode_default" => SettingsAgentClass::OpenCode,
            _ => return None,
        };
        Some(self.settings_context(class))
    }

    pub(super) fn settings_agent_for_context(&self, context: &str) -> Option<SettingsAgentClass> {
        if context
            .strip_prefix("agent-context/hermes/sha256:")
            .is_some_and(|id| {
                id.len() == 64
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            })
        {
            return Some(SettingsAgentClass::Hermes);
        }
        [
            SettingsAgentClass::Codex,
            SettingsAgentClass::CodexProfile,
            SettingsAgentClass::Claude,
            SettingsAgentClass::Qoder,
            SettingsAgentClass::Pi,
            SettingsAgentClass::Dsh,
            SettingsAgentClass::DshDesktop,
            SettingsAgentClass::OpenCode,
        ]
        .into_iter()
        .find(|class| context == self.settings_context(*class))
    }

    pub(super) fn settings_context(&self, class: SettingsAgentClass) -> String {
        let Some(path) = class.native_path(&self.scanner) else {
            return String::new();
        };
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
        match self.capture_settings_facts_once(spec) {
            Err(ControlReadError::SnapshotChanged) => self.capture_settings_facts_once(spec),
            result => result,
        }
    }

    fn capture_settings_facts_once(
        &self,
        spec: &AgentSettingsSpecV2,
    ) -> Result<AgentSettingsPlanningInput, ControlReadError> {
        let class = self
            .settings_agent_for_context(&spec.context_id)
            .ok_or(ControlReadError::NotFound)?;
        self.check_codex_slot(spec, class)?;
        // Desktop model access does not establish independent Skill/Worker support.
        if class == SettingsAgentClass::DshDesktop && !spec.keeps_skills() {
            return Err(ControlReadError::Denied);
        }
        // Probe the installed composition once per facts capture, not once per
        // native-file snapshot. Apply captures again under the admission lock;
        // mutable configuration still participates in both snapshots below.
        #[cfg(any(unix, windows))]
        let dsh_composition = if matches!(
            class,
            SettingsAgentClass::Dsh | SettingsAgentClass::DshDesktop
        ) && matches!(spec.model, AgentFacetIntent::Configure { .. })
        {
            Some(
                (if class == SettingsAgentClass::DshDesktop {
                    self.scanner.check_dsh_desktop_model_configuration()
                } else {
                    self.scanner.check_dsh_model_configuration()
                })
                .map_err(|_| ControlReadError::Unavailable)?,
            )
        } else {
            None
        };
        let first = self.complete_settings_snapshot(spec, class)?;
        let mut second = self.complete_settings_snapshot(spec, class)?;
        #[cfg(any(unix, windows))]
        if let Some(checked) = dsh_composition.as_ref()
            && !(if class == SettingsAgentClass::DshDesktop {
                self.scanner
                    .dsh_desktop_model_configuration_is_current(checked)
            } else {
                self.scanner.dsh_model_configuration_is_current(checked)
            })
            .map_err(|_| ControlReadError::SnapshotChanged)?
        {
            return Err(ControlReadError::SnapshotChanged);
        }
        if first.facts.dependency_digest != second.facts.dependency_digest {
            return Err(ControlReadError::SnapshotChanged);
        }
        #[cfg(any(unix, windows))]
        if let Some(checked) = dsh_composition {
            // The admitted proof is part of Preview/Apply authority, not just a
            // transient success check. A new valid runtime still needs fresh approval.
            let previous = second.facts.dependency_digest.clone();
            let digest =
                CanonicalDigest::of(&("hiroute.dsh-composition-facts/v1", &previous, checked))
                    .map_err(|_| ControlReadError::Corrupt)?;
            second
                .facts
                .capabilities
                .rebind_dependency(&previous, &digest);
            second.facts.dependency_digest = digest;
        }
        Ok(second)
    }

    fn settings_snapshot(
        &self,
        spec: &AgentSettingsSpecV2,
        class: SettingsAgentClass,
    ) -> Result<AgentSettingsPlanningInput, ControlReadError> {
        let diagnostics = self.agent_read_diagnostics();
        let _total = Timer::start(&diagnostics, Category::SettingsPlanning, Stage::Total);
        if matches!(
            class,
            SettingsAgentClass::Qoder
                | SettingsAgentClass::Pi
                | SettingsAgentClass::Dsh
                | SettingsAgentClass::DshDesktop
                | SettingsAgentClass::OpenCode
        ) {
            return self.attach_additional_model_facts(
                spec,
                class,
                self.additional_settings_snapshot(spec, class)?,
            );
        }
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
        // Both Skill facets consume the same explicitly observed native loading capability.
        // Model-only edits remain independent from that optional check.
        let configure_skill = matches!(spec.collaboration, AgentFacetIntent::Configure { .. })
            || matches!(
                spec.management_skill,
                Some(AgentFacetIntent::Configure { .. })
            );
        let discovery = if class == SettingsAgentClass::Claude {
            self.scanner.claude_settings_discovery(configure_skill)
        } else if class == SettingsAgentClass::CodexProfile {
            self.scanner.codex_profile_settings_discovery()
        } else {
            self.scanner.codex_settings_discovery(configure_skill)
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
            && configure_skill
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
            if class == SettingsAgentClass::CodexProfile {
                hiroute_integrations::CODEX_STANDALONE_PROFILE_ID.to_owned()
            } else {
                installation.profile.profile_id.clone()
            },
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

        // Collaboration belongs to the original home, not the standalone model profile.
        let skill_target = class
            .skill_target()
            .map_err(|_| ControlReadError::Corrupt)?;
        let (skill_before_fingerprint, observed_skill_file) =
            if matches!(spec.collaboration, AgentFacetIntent::Keep) {
                (None, None)
            } else {
                let fingerprint = self
                    .artifacts
                    .current_external_fingerprint(&skill_target)
                    .map_err(super::map_port)?;
                let bytes = self
                    .artifacts
                    .read_native_target(&skill_target)
                    .map_err(super::map_port)?;
                if fingerprint
                    != self
                        .artifacts
                        .current_external_fingerprint(&skill_target)
                        .map_err(super::map_port)?
                {
                    return Err(ControlReadError::SnapshotChanged);
                }
                (fingerprint, bytes.map(|b| CanonicalDigest::of_bytes(&b)))
            };

        let runtime = self
            .managed_agent_runtime
            .lock()
            .map_err(|_| ControlReadError::Unavailable)?
            .clone()
            .ok_or(ControlReadError::Unavailable)?;
        let waiting = Timer::start(&diagnostics, Category::SettingsPlanning, Stage::LockWait);
        let stores = self.stores_lock().map_err(super::map_port)?;
        drop(waiting);
        let held = Timer::start(&diagnostics, Category::SettingsPlanning, Stage::LockHeld);
        let workspace = WorkspaceId::default();
        // New settings saves do not manage startup. Keep the preview field for V2 clients;
        // only an older, journal-owned login item may require cleanup on final restore.
        let other_context = match class {
            SettingsAgentClass::Codex | SettingsAgentClass::CodexProfile => {
                SettingsAgentClass::Claude
            }
            SettingsAgentClass::Claude => SettingsAgentClass::Codex,
            SettingsAgentClass::Qoder
            | SettingsAgentClass::Pi
            | SettingsAgentClass::Dsh
            | SettingsAgentClass::DshDesktop
            | SettingsAgentClass::OpenCode
            | SettingsAgentClass::Hermes => {
                return Err(ControlReadError::Denied);
            }
        };
        let connection_id = format!("agent-connection/{}", spec.context_id);
        let other_connection_id =
            format!("agent-connection/{}", self.settings_context(other_context));
        let login_item_required = false;
        // Releasing the last managed connection also releases the login item this feature
        // owns. Ownership is proven only by journaled evidence — the newest login-item
        // declaration across succeeded settings operations: an establishment this feature
        // created owns the item, a removal releases it, and a pre-existing user item
        // (established without creation) is never owned and never removed.
        let copy = Timer::start(
            &diagnostics,
            Category::SettingsPlanning,
            Stage::SnapshotCopy,
        );
        let (
            (
                expected_revisions,
                captured_publication,
                current_grant,
                other_grant,
                token_input_fingerprint,
                expected_grant_generation,
                skill_before,
                resident,
            ),
            stamp,
        ) = stores
            .capture_agent_read(true, |control, secret| {
                let unavailable =
                    || PortError::new(PortErrorCode::Unavailable, "agent.settings.read");
                let denied =
                    || PortError::new(PortErrorCode::PermissionDenied, "agent.settings.input");
                let secret = secret.ok_or_else(unavailable)?;
                let revisions = control.current_revisions(&workspace)?;
                let publication = control.capture_active_publication(&workspace)?;
                let mut auth = Timer::start(
                    &diagnostics,
                    Category::SettingsPlanning,
                    Stage::GrantAuthentication,
                );
                auth.counts.grant_authentications = 1;
                let current =
                    secret.inspect_agent_access_grant(workspace.as_str(), &connection_id)?;
                drop(auth);
                let mut auth = Timer::start(
                    &diagnostics,
                    Category::SettingsPlanning,
                    Stage::GrantAuthentication,
                );
                auth.counts.grant_authentications = 1;
                let other =
                    secret.inspect_agent_access_grant(workspace.as_str(), &other_connection_id)?;
                drop(auth);
                let token = match &spec.access_token {
                    AgentAccessTokenIntentV1::Keep => None,
                    AgentAccessTokenIntentV1::Regenerate if current.is_some() => None,
                    AgentAccessTokenIntentV1::Set { input_slot } if current.is_some() => {
                        let inputs = self.agent_token_inputs.lock().map_err(|_| unavailable())?;
                        let input = inputs.get(input_slot).ok_or_else(|| {
                            PortError::new(PortErrorCode::NotFound, "agent.settings.input")
                        })?;
                        if !hiroute_domain::valid_user_agent_token(input.expose()) {
                            return Err(denied());
                        }
                        Some(secret.fingerprint(input)?)
                    }
                    _ => return Err(denied()),
                };
                let generation =
                    secret.agent_access_grant_generation(workspace.as_str(), &connection_id)?;
                let skill = control.skill_installation(&workspace, class.skill_root_ref())?;
                let resident = if !runtime.resident_service_ready
                    && current.is_some()
                    && other.is_none()
                    && matches!(spec.model, AgentFacetIntent::Restore { .. })
                {
                    let mut locator =
                        Timer::start(&diagnostics, Category::ResidentService, Stage::Locator);
                    let capture = control.capture_agent_locator(
                        &workspace,
                        hiroute_local_storage::AgentLocatorScope::ResidentService,
                    )?;
                    super::agent_read_diagnostics::count_locator(&mut locator, &capture);
                    Some(capture)
                } else {
                    None
                };
                Ok((
                    revisions,
                    publication,
                    current,
                    other,
                    token,
                    generation,
                    skill,
                    resident,
                ))
            })
            .map_err(super::map_port)?;
        drop(copy);
        if captured_publication.is_none() && !matches!(spec.model, AgentFacetIntent::Keep) {
            return Err(ControlReadError::NotFound);
        }
        let publication_identity = captured_publication
            .as_ref()
            .map(|record| record.identity());
        let resident_witness = resident
            .as_ref()
            .map(|capture| capture.witness())
            .transpose()
            .map_err(super::map_port)?;

        let mut restore_points = BTreeMap::new();
        let mut active_configuration = None;
        let mut active_model_selection = None;
        let mut active_protected_native_ids = Vec::new();
        let mut selected_restore = None;
        let mut selected_collaboration_restore = None;
        for original in stores
            .control()
            .succeeded_agent_operations_for_kind(&workspace, "ApplyAgentConnectionChange")
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
                SettingsAgentClass::Qoder | SettingsAgentClass::Pi | SettingsAgentClass::Dsh | SettingsAgentClass::DshDesktop | SettingsAgentClass::Hermes | SettingsAgentClass::OpenCode => return Err(ControlReadError::Denied),
                SettingsAgentClass::Codex | SettingsAgentClass::CodexProfile => original
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
                .grant_effects()
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
                skills::skill_restore_ref(
                    &spec.context_id,
                    AgentSettingsFacet::Collaboration,
                    &operation,
                ),
                AgentSettingsFacet::Collaboration,
            );
            if matches!(spec.model, AgentFacetIntent::Keep) {
                restore_points.insert(
                    codex_model_restore_point_ref(&operation),
                    AgentSettingsFacet::Collaboration,
                );
            }
        }
        drop(held);
        drop(stores);
        let publication = captured_publication
            .map(|record| record.verify())
            .transpose()
            .map_err(super::map_port)?;
        let active = publication
            .as_ref()
            .map(|record| record.verify().map_err(|_| ControlReadError::Corrupt))
            .transpose()?;
        let login_item_removal_required = match resident {
            Some(capture) => {
                let _verify = Timer::start(&diagnostics, Category::ResidentService, Stage::Verify);
                login_item_owned_by_this_feature(capture)?
            }
            None => false,
        };
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
        let template = collaboration_template(spec)?;
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
                if class.is_codex() || !settings.fixed_models().is_empty() =>
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
        } else if class.is_codex() {
            self.scanner.codex_catalog_summary().ok()
        } else {
            None
        };
        let native_default_model = match class {
            SettingsAgentClass::Qoder
            | SettingsAgentClass::Pi
            | SettingsAgentClass::Dsh
            | SettingsAgentClass::DshDesktop
            | SettingsAgentClass::OpenCode
            | SettingsAgentClass::Hermes => {
                return Err(ControlReadError::Denied);
            }
            SettingsAgentClass::Codex | SettingsAgentClass::CodexProfile => codex_catalog_summary
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
        let original_native_ids = if class.is_codex()
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
                    .preserved_codex_model_selections(
                        catalog,
                        &mut fixed_candidate_facts,
                        active_configuration
                            .as_ref()
                            .filter(|_| class == SettingsAgentClass::Codex)
                            .map(|op| (op, target.as_str())),
                    )
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
        let mut model_catalog = if let (
            SettingsAgentClass::Codex | SettingsAgentClass::CodexProfile,
            AgentFacetIntent::Configure { settings },
        ) = (class, &spec.model)
        {
            let baseline = if class == SettingsAgentClass::CodexProfile {
                hiroute_integrations::CodexCatalogBaseline::Scope
            } else {
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
            if let hiroute_domain::AgentModelSelectionV2::CodexDefault { fixed_models, .. } =
                &mut complete
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
        let codex_context_override = class.is_codex()
            && matches!(&spec.model, AgentFacetIntent::Configure { settings } if !settings.allowed_plan_ids().is_empty())
            && hiroute_integrations::codex_has_context_override(
                &hiroute_integrations::CodexConfigurationScope::user_file(
                    self.scanner.codex_user_config_target(),
                ),
            )
            .map_err(|_| ControlReadError::Unavailable)?;
        let claude_plan_capability_unavailable = if let (
            SettingsAgentClass::Claude,
            AgentFacetIntent::Configure { settings },
            Some(publication),
        ) = (class, &spec.model, active.as_ref())
        {
            settings.allowed_plan_ids().iter().any(|id| publication.plans.iter().find(|plan| plan.agent_plan_id() == id).is_none_or(|plan| {
                let Ok(plan) = plan.clone().into_current() else { return true; };
                matches!(hiroute_integrations::agents::claude_plan_capability_preview(&plan), hiroute_application_api::ClaudeClientCapabilityPreviewV1::Unavailable { reason } if reason != "context_window_below_minimum")
            }))
        } else {
            false
        };
        let claude_context_override = class == SettingsAgentClass::Claude
            && matches!(&spec.model, AgentFacetIntent::Configure { settings } if !settings.allowed_plan_ids().is_empty())
            && self
                .scanner
                .claude_context_override()
                .map_err(|_| ControlReadError::Unavailable)?;
        {
            let waiting = Timer::start(&diagnostics, Category::SettingsPlanning, Stage::LockWait);
            let stores = self.stores_lock().map_err(super::map_port)?;
            drop(waiting);
            let _held = Timer::start(&diagnostics, Category::SettingsPlanning, Stage::LockHeld);
            let _fence = Timer::start(&diagnostics, Category::SettingsPlanning, Stage::Fence);
            stores
                .fence_agent_read(&stamp, |control, secret| {
                    let changed =
                        || PortError::new(PortErrorCode::Conflict, "agent.settings.read-fence");
                    let secret = secret.ok_or_else(changed)?;
                    if control.current_revisions(&workspace)? != expected_revisions
                        || control.active_publication_identity(&workspace)? != publication_identity
                        || control.skill_installation(&workspace, class.skill_root_ref())?
                            != skill_before
                        || !secret.current_agent_grant_matches(
                            workspace.as_str(),
                            &connection_id,
                            current_grant.as_ref(),
                        )?
                        || !secret.current_agent_grant_matches(
                            workspace.as_str(),
                            &other_connection_id,
                            other_grant.as_ref(),
                        )?
                        || secret
                            .agent_access_grant_generation(workspace.as_str(), &connection_id)?
                            != expected_grant_generation
                    {
                        return Err(changed());
                    }
                    if let Some(witness) = &resident_witness
                        && !witness.matches_current(control)?
                    {
                        return Err(changed());
                    }
                    if let AgentAccessTokenIntentV1::Set { input_slot } = &spec.access_token {
                        let inputs = self.agent_token_inputs.lock().map_err(|_| changed())?;
                        let input = inputs.get(input_slot).ok_or_else(changed)?;
                        if Some(secret.fingerprint(input)?) != token_input_fingerprint {
                            return Err(changed());
                        }
                    }
                    Ok(())
                })
                .map_err(super::map_port)?;
        }
        let dependency_digest = CanonicalDigest::of(&serde_json::json!({
            "codex_profile_dependencies": self.codex_profile_dependencies(class, spec, active_configuration.as_ref())?,
            "claude_context_override": claude_context_override,
            "claude_plan_capability_unavailable": claude_plan_capability_unavailable,
            "codex_context_override": codex_context_override,
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
            "restore_inherits_root":class == SettingsAgentClass::CodexProfile,
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
            SettingsAgentClass::Qoder
            | SettingsAgentClass::Pi
            | SettingsAgentClass::Dsh
            | SettingsAgentClass::DshDesktop
            | SettingsAgentClass::OpenCode
            | SettingsAgentClass::Hermes => {
                return Err(ControlReadError::Denied);
            }
            SettingsAgentClass::Codex | SettingsAgentClass::CodexProfile => {
                SettingsModelTargetFacts::Codex {
                    provider_id: "hiroute".into(),
                    endpoint: runtime.gateway_base_url.clone(),
                }
            }
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
            additional_skill_file: None,
            collaboration_state,
            expected_revisions,
            subject,
            model_file: Some(SettingsModelFileFacts {
                expected_content,
                before_fingerprint,
                publication_digest: publication.map(|record| record.digest),
                expected_grant_generation,
                token_input_fingerprint,
                active_configuration,
                restore: selected_restore,
                target: model_target,
            }),
            skill_file: SettingsSkillFileFacts {
                root_ref: class.skill_root_ref().into(),
                target: skill_target,
                before: skill_before,
                observed_file: observed_skill_file,
                before_fingerprint: skill_before_fingerprint,
                template,
            },
            facts: AgentSettingsFacts {
                reference_only_skill_restores: BTreeSet::new(),
                management_file_conflict: false,
                native_target: None,
                context_id: spec.context_id.clone(),
                dependency_digest,
                capabilities,
                collaboration_file_conflict,
                restore_points,
                model: Some(SettingsModelFacts {
                    common: SettingsModelCommonFacts {
                        ingress: installation
                            .profile
                            .model_connection()
                            .map_err(|_| ControlReadError::Corrupt)?
                            .ok_or(ControlReadError::Denied)?
                            .ingress_protocol,
                        available_surfaces: self.scanner.available_model_surfaces(class.agent_id()),
                        model_publication: active,
                        login_item_required,
                        login_item_removal_required,
                        fixed_candidate_facts,
                    },
                    native: if class.is_codex() {
                        SettingsModelNativeFacts::Codex(Box::new(SettingsCodexModelFacts {
                            context_override: codex_context_override,
                            model_catalog,
                            preserved_codex_models,
                            preserved_codex_bindings,
                            required_native_model_ids,
                            unproven_native_model_ids,
                            require_native_model_routes: preserve_native_models,
                            native_default_must_be_original,
                            native_default_model,
                            restore_native_model_ids,
                            restore_inherits_root: class == SettingsAgentClass::CodexProfile,
                            restored_native_model,
                        }))
                    } else {
                        SettingsModelNativeFacts::Claude(SettingsClaudeNativeFacts {
                            context_override: claude_context_override,
                            plan_capability_unavailable: claude_plan_capability_unavailable,
                            native_default_model,
                            presets: native_claude_presets,
                        })
                    },
                }),
            },
        })
    }
}

/// The newest journaled login-item declaration decides whether this feature owns the login
/// item: an establishment this feature created owns it, a removal releases it, and a
/// pre-existing user item is never owned. Succeeded operations are visited newest first.
fn login_item_owned_by_this_feature(
    capture: hiroute_local_storage::CapturedAgentLocator,
) -> Result<bool, ControlReadError> {
    match capture {
        hiroute_local_storage::CapturedAgentLocator::Found(found) => {
            let original = found.verify().map_err(super::map_port)?;
            let intent = original
                .plan
                .external()
                .iter()
                .find(|intent| is_settings_login_item(intent))
                .ok_or(ControlReadError::Corrupt)?;
            Ok(decode_settings_login_item(intent)
                .map_err(super::map_port)?
                .created())
        }
        hiroute_local_storage::CapturedAgentLocator::Pending(pending, _) => {
            pending.verify().map_err(super::map_port)?;
            Err(ControlReadError::SnapshotChanged)
        }
        hiroute_local_storage::CapturedAgentLocator::AbsentWithCompleteCoverage(_) => Ok(false),
    }
}

fn collaboration_template(
    spec: &AgentSettingsSpecV2,
) -> Result<CollaborationSkillTemplate, ControlReadError> {
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
    CollaborationSkillTemplate::bundled(template_revision, template_content)
        .map_err(|_| ControlReadError::Corrupt)
}
