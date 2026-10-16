//! Explicit Hermes scopes and independent management Skill facts.
use super::*;
use hiroute_domain::{AgentModelSelectionV2, AgentNativeTargetV1};

impl LocalControlAdapter {
    pub(in crate::control::runtime) fn hermes_target_for_spec(
        &self,
        spec: &AgentSettingsSpecV2,
    ) -> Result<AgentNativeTargetV1, ControlReadError> {
        self.hermes_target_for_context(&spec.context_id, spec.native_target.as_ref())
    }

    pub(in crate::control::runtime) fn hermes_target_for_context(
        &self,
        context_id: &str,
        supplied: Option<&AgentNativeTargetV1>,
    ) -> Result<AgentNativeTargetV1, ControlReadError> {
        let target = if let Some(target) = supplied {
            hiroute_integrations::canonical_hermes_target(target)
                .map_err(|_| ControlReadError::Denied)?
        } else {
            self.stores_lock()
                .map_err(super::super::map_port)?
                .control()
                .agent_settings_native_targets(&WorkspaceId::default())
                .map_err(super::super::map_port)?
                .into_iter()
                .find(|target| {
                    hiroute_integrations::hermes_context_id(target.config_root()) == context_id
                })
                .ok_or(ControlReadError::NotFound)?
        };
        if hiroute_integrations::hermes_context_id(target.config_root()) != context_id {
            return Err(ControlReadError::Denied);
        }
        let bindings = hiroute_integrations::hermes_external_targets(target.config_root())
            .map_err(|_| ControlReadError::Corrupt)?;
        self.artifacts
            .register_external_targets(bindings)
            .map_err(|_| ControlReadError::Denied)?;
        self.remember_hermes_scan_target(&target)?;
        Ok(target)
    }

    pub(super) fn hermes_settings_snapshot(
        &self,
        spec: &AgentSettingsSpecV2,
    ) -> Result<AgentSettingsPlanningInput, ControlReadError> {
        let target_scope = self.hermes_target_for_spec(spec)?;
        let root_ref = format!("skill-root/{}", spec.context_id);
        let installation = if matches!(spec.model, AgentFacetIntent::Keep) {
            hiroute_integrations::hermes_management_installation(&target_scope)
        } else if let AgentFacetIntent::Configure {
            settings: AgentModelSelectionV2::HermesDefault { protocol, .. },
        } = &spec.model
        {
            hiroute_integrations::hermes_settings_installation_for_protocol(
                &target_scope,
                *protocol,
            )
        } else {
            hiroute_integrations::hermes_settings_installation(&target_scope)
        }
        .map_err(|_| ControlReadError::Denied)?;
        let subject = hiroute_integrations::hermes_subject(target_scope.config_root())
            .map_err(|_| ControlReadError::Corrupt)?;
        let target = AgentConnectionEffectRoleV1::RoutingSkill
            .settings_target_for(&subject)
            .map_err(|_| ControlReadError::Corrupt)?;
        // Keep does not acquire or validate the other facet's native file.
        let (before_fingerprint, observed_file) =
            if matches!(spec.skill_intent(), AgentFacetIntent::Keep) {
                (None, None)
            } else {
                let before_fingerprint = self
                    .artifacts
                    .current_external_fingerprint(&target)
                    .map_err(super::super::map_port)?;
                let observed_file = self
                    .artifacts
                    .read_native_target(&target)
                    .map_err(super::super::map_port)?
                    .as_deref()
                    .map(|bytes| CanonicalDigest::of_bytes(bytes.as_slice()));
                if before_fingerprint
                    != self
                        .artifacts
                        .current_external_fingerprint(&target)
                        .map_err(super::super::map_port)?
                {
                    return Err(ControlReadError::SnapshotChanged);
                }
                (before_fingerprint, observed_file)
            };
        let stores = self.stores_lock().map_err(super::super::map_port)?;
        let workspace = WorkspaceId::default();
        let expected_revisions = stores
            .control()
            .current_revisions(&workspace)
            .map_err(super::super::map_port)?;
        let before = stores
            .control()
            .skill_installation(&workspace, &root_ref)
            .map_err(super::super::map_port)?;
        let mut restore_points = BTreeMap::new();
        if before
            .as_ref()
            .is_some_and(|record| record.contexts.contains(&spec.context_id))
        {
            for operation in stores
                .control()
                .succeeded_agent_operations_for_kind(&workspace, "ApplyAgentConnectionChange")
                .map_err(super::super::map_port)?
            {
                if operation.plan.spec().command_id != "agents.settings.apply"
                    || operation.plan.spec().resource_id.as_deref() != Some(&spec.context_id)
                {
                    continue;
                }
                let original: AgentSettingsSpecV2 =
                    serde_json::from_value(operation.plan.spec().desired_state.clone())
                        .map_err(|_| ControlReadError::Corrupt)?;
                if original.context_id == spec.context_id
                    && matches!(original.skill_intent(), AgentFacetIntent::Configure { .. })
                {
                    restore_points.insert(
                        super::skills::skill_restore_ref(
                            &spec.context_id,
                            AgentSettingsFacet::ManagementSkill,
                            &operation.operation_id,
                        ),
                        AgentSettingsFacet::ManagementSkill,
                    );
                    if matches!(spec.model, AgentFacetIntent::Keep) {
                        restore_points.insert(
                            codex_model_restore_point_ref(&operation.operation_id),
                            AgentSettingsFacet::ManagementSkill,
                        );
                    }
                    break;
                }
            }
        }
        drop(stores);
        let template = management_template()?;
        let collaboration_file_conflict = match spec.skill_intent() {
            AgentFacetIntent::Configure { .. } => plan_skill_install(
                &root_ref,
                &spec.context_id,
                &template,
                before.as_ref(),
                observed_file.as_ref(),
            )
            .is_err(),
            AgentFacetIntent::Restore { .. } => before.as_ref().is_some_and(|record| {
                plan_skill_remove(&spec.context_id, record, observed_file.as_ref())
                    .map_or(true, |plan| plan.file_action == SkillFileAction::Conflict)
            }),
            AgentFacetIntent::Keep => false,
        };
        let collaboration_state = serde_json::Value::Null;
        let releases_borrowed_reference =
            matches!(spec.skill_intent(), AgentFacetIntent::Restore { .. })
                && before.as_ref().is_some_and(|record| {
                    record.file_ownership
                        == hiroute_domain::CollaborationSkillFileOwnership::BorrowedIdentical
                        && record.contexts.contains(&spec.context_id)
                });
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
        let dependency_digest = CanonicalDigest::of(&serde_json::json!({
            "schema":"hermes-management-facts/v1",
            "context":spec.context_id,
            "observation":installation.observation_digest,
            "capabilities":semantic_evidence,
            "skill_content":if releases_borrowed_reference { None } else { observed_file.as_ref() },
            "skill_fingerprint":if releases_borrowed_reference { None } else { before_fingerprint.as_ref() },
            "skill_record":before,"skill_template":template.digest,
            "revisions":expected_revisions,"restores":restore_points,
            "worker_channel":"local_trust_v1","collaboration_state":collaboration_state,
            "collaboration_file_conflict":collaboration_file_conflict,
        })).map_err(|_| ControlReadError::Corrupt)?;
        let capabilities =
            AgentCapabilitySet::new(installation.capability_evidence.iter().cloned().map(
                |mut item| {
                    if item.dependency_digest == installation.observation_digest {
                        item.dependency_digest = dependency_digest.clone();
                    }
                    item
                },
            ))
            .map_err(|_| ControlReadError::Corrupt)?;
        Ok(AgentSettingsPlanningInput {
            additional_skill_file: None,
            collaboration_state,
            expected_revisions,
            subject,
            model_file: None,
            skill_file: SettingsSkillFileFacts {
                root_ref,
                target,
                before,
                observed_file,
                before_fingerprint,
                template,
            },
            facts: AgentSettingsFacts {
                reference_only_skill_restores: BTreeSet::new(),
                management_file_conflict: collaboration_file_conflict,
                native_target: Some(target_scope),
                context_id: spec.context_id.clone(),
                dependency_digest,
                capabilities,
                collaboration_file_conflict: false,
                restore_points,
                model: None,
            },
        })
    }
}

pub(super) fn management_template() -> Result<CollaborationSkillTemplate, ControlReadError> {
    CollaborationSkillTemplate::bundled(
        "shared-management-v2",
        include_str!("../../../../../../assets/skills/hiroute-management/SKILL.md"),
    )
    .map_err(|_| ControlReadError::Corrupt)
}
