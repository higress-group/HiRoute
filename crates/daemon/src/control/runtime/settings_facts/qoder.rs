//! Collaboration facts deliberately exclude native models, login material and model grants.
use super::*;
use hiroute_integrations::QoderCollaborationProbeTarget;

impl LocalControlAdapter {
    pub(super) fn additional_settings_snapshot(
        &self,
        spec: &AgentSettingsSpecV2,
        class: SettingsAgentClass,
    ) -> Result<AgentSettingsPlanningInput, ControlReadError> {
        let discovery = if class == SettingsAgentClass::Dsh {
            self.scanner.dsh_settings_discovery()
        } else if class == SettingsAgentClass::Pi {
            self.scanner.pi_settings_discovery()
        } else {
            self.scanner.qoder_settings_discovery(matches!(
                spec.collaboration,
                AgentFacetIntent::Configure { .. }
            ))
        };
        let AgentDiscoveryOutcomeV1::Supported { installation } = discovery.outcome else {
            return Err(ControlReadError::NotFound);
        };
        if installation.agent_id != class.agent_id() {
            return Err(ControlReadError::Corrupt);
        }
        let subject = AgentConnectionTransactionSubjectV1::from_registered_profile(
            installation.agent_id.clone(),
            installation.profile.profile_id.clone(),
            installation.profile.integration_profile_ref.clone(),
        )
        .map_err(|_| ControlReadError::Corrupt)?;
        let target = class
            .skill_target()
            .map_err(|_| ControlReadError::Corrupt)?;
        // Keep does not acquire or validate the other facet's native file.
        let (before_fingerprint, observed_file) =
            if matches!(spec.collaboration, AgentFacetIntent::Keep) {
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
            .skill_installation(&workspace, class.skill_root_ref())
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
                    && matches!(original.collaboration, AgentFacetIntent::Configure { .. })
                {
                    restore_points.insert(
                        codex_model_restore_point_ref(&operation.operation_id),
                        AgentSettingsFacet::Collaboration,
                    );
                    break;
                }
            }
        }
        drop(stores);
        let template = collaboration_template(spec)?;
        let collaboration_file_conflict = match spec.collaboration {
            AgentFacetIntent::Configure { .. } => plan_skill_install(
                class.skill_root_ref(),
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
        let collaboration_state = self.settings_collaboration_state(spec)?;
        let releases_borrowed_reference =
            matches!(spec.collaboration, AgentFacetIntent::Restore { .. })
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
            "schema":"qoder-collaboration-facts/v1",
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
            collaboration_state,
            expected_revisions,
            subject,
            model_file: None,
            skill_file: SettingsSkillFileFacts {
                root_ref: class.skill_root_ref().into(),
                target,
                before,
                observed_file,
                before_fingerprint,
                template,
            },
            facts: AgentSettingsFacts {
                context_id: spec.context_id.clone(),
                dependency_digest,
                capabilities,
                collaboration_file_conflict,
                restore_points,
                model: None,
            },
        })
    }

    /// Persisted ownership selects the verification phase. A changed or missing owned file
    /// must fail installed verification, never silently fall back to a temporary probe.
    pub(in crate::control::runtime) fn qoder_collaboration_probe_target(
        &self,
    ) -> Result<QoderCollaborationProbeTarget, ControlReadError> {
        let class = SettingsAgentClass::Qoder;
        let context = self.settings_context(class);
        let record = self
            .stores_lock()
            .map_err(super::super::map_port)?
            .control()
            .skill_installation(&WorkspaceId::default(), class.skill_root_ref())
            .map_err(super::super::map_port)?;
        Ok(match record {
            Some(record) if record.contexts.contains(&context) => {
                QoderCollaborationProbeTarget::InstalledUserSkill {
                    expected_content: record.content_digest,
                }
            }
            _ => QoderCollaborationProbeTarget::PreinstallCapability,
        })
    }
}
