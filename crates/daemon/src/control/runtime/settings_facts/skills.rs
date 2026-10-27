//! Independent native Skill resources over the existing ownership store and Operation writer.
use super::*;
use hiroute_domain::{AgentConnectionEffectRoleV1, AgentConnectionTransactionSubjectV1};

pub(in crate::control::runtime) fn skill_restore_ref(
    _context: &str,
    facet: AgentSettingsFacet,
    operation: &hiroute_domain::OperationId,
) -> String {
    let name = match facet {
        AgentSettingsFacet::ManagementSkill => "management",
        AgentSettingsFacet::Collaboration => "collaboration",
        AgentSettingsFacet::Model => "model",
    };
    format!("{}/{}", codex_model_restore_point_ref(operation), name)
}

impl LocalControlAdapter {
    pub(in crate::control::runtime) fn settings_skill_resource(
        &self,
        context: &str,
        supplied: Option<&hiroute_domain::AgentNativeTargetV1>,
        facet: AgentSettingsFacet,
    ) -> Result<(String, String), ControlReadError> {
        let class = self
            .settings_agent_for_context(context)
            .ok_or(ControlReadError::NotFound)?;
        let legacy = if class == SettingsAgentClass::Hermes {
            AgentSettingsFacet::ManagementSkill
        } else {
            AgentSettingsFacet::Collaboration
        };
        let root = if class == SettingsAgentClass::Hermes {
            format!("skill-root/{context}")
        } else {
            class.skill_root_ref().to_owned()
        };
        let root = if facet == legacy {
            root
        } else {
            format!("{root}/additional")
        };
        let role = AgentConnectionEffectRoleV1::settings_skill(context, facet)
            .ok_or(ControlReadError::Denied)?;
        let target = if class == SettingsAgentClass::Hermes {
            let target = self.hermes_target_for_context(context, supplied)?;
            role.settings_target_for(
                &hiroute_integrations::hermes_subject(target.config_root())
                    .map_err(|_| ControlReadError::Corrupt)?,
            )
        } else {
            class.skill_target_for(facet)
        }
        .map_err(|_| ControlReadError::Corrupt)?;
        Ok((root, target))
    }

    pub(super) fn complete_settings_snapshot(
        &self,
        spec: &AgentSettingsSpecV2,
        class: SettingsAgentClass,
    ) -> Result<AgentSettingsPlanningInput, ControlReadError> {
        let mut input = if matches!(spec.model, AgentFacetIntent::Keep) {
            self.skill_only_settings_snapshot(spec, class)?
        } else if class == SettingsAgentClass::Hermes {
            self.attach_hermes_model_facts(spec, self.hermes_settings_snapshot(spec)?)?
        } else {
            self.settings_snapshot(spec, class)?
        };
        let facet = if spec.skill_facet() == AgentSettingsFacet::ManagementSkill {
            AgentSettingsFacet::Collaboration
        } else {
            AgentSettingsFacet::ManagementSkill
        };
        if matches!(spec.skill_intent_for(facet), AgentFacetIntent::Keep) {
            return self.finish_skill_snapshot(spec, input);
        }
        let (file, conflict, restores) = self.capture_skill_resource(spec, facet)?;
        let previous = input.facts.dependency_digest.clone();
        let digest = CanonicalDigest::of(&serde_json::json!({
            "schema":"agent-independent-skills-facts/v1", "primary":previous,
            "additional":{"root":file.root_ref,"target":file.target,"before":file.before,
                "observed":file.observed_file,"fingerprint":file.before_fingerprint,"template":file.template.digest,
                "conflict":conflict,"restores":restores}
        })).map_err(|_| ControlReadError::Corrupt)?;
        input
            .facts
            .capabilities
            .rebind_dependency(&previous, &digest);
        input.facts.dependency_digest = digest;
        input.facts.restore_points.extend(restores);
        match facet {
            AgentSettingsFacet::ManagementSkill => input.facts.management_file_conflict = conflict,
            AgentSettingsFacet::Collaboration => input.facts.collaboration_file_conflict = conflict,
            _ => return Err(ControlReadError::Denied),
        }
        input.additional_skill_file = Some(file);
        self.finish_skill_snapshot(spec, input)
    }

    fn finish_skill_snapshot(
        &self,
        spec: &AgentSettingsSpecV2,
        mut input: AgentSettingsPlanningInput,
    ) -> Result<AgentSettingsPlanningInput, ControlReadError> {
        let facets = [
            AgentSettingsFacet::ManagementSkill,
            AgentSettingsFacet::Collaboration,
        ];
        for facet in facets {
            if !matches!(
                spec.skill_intent_for(facet),
                AgentFacetIntent::Restore { .. }
            ) {
                continue;
            }
            let file = if facet == spec.skill_facet() {
                Some(&input.skill_file)
            } else {
                input.additional_skill_file.as_ref()
            };
            if file.and_then(|f| f.before.as_ref()).is_some_and(|r| {
                r.file_ownership
                    == hiroute_domain::CollaborationSkillFileOwnership::BorrowedIdentical
                    && r.contexts.contains(&spec.context_id)
            }) {
                input.facts.reference_only_skill_restores.insert(facet);
            }
        }
        let changed: BTreeSet<_> = facets
            .into_iter()
            .filter(|facet| !matches!(spec.skill_intent_for(*facet), AgentFacetIntent::Keep))
            .collect();
        if matches!(spec.model, AgentFacetIntent::Keep)
            && !changed.is_empty()
            && changed == input.facts.reference_only_skill_restores
        {
            // External content is not an Apply precondition for releasing only our references.
            let previous = input.facts.dependency_digest.clone();
            let digest = CanonicalDigest::of(&serde_json::json!({
                "schema":"borrowed-skill-reference-release/v1","spec":spec,
                "revisions":input.expected_revisions,"restores":input.facts.restore_points,
                "primary":input.skill_file.before,"additional":input.additional_skill_file.as_ref().and_then(|f| f.before.as_ref()),
            })).map_err(|_| ControlReadError::Corrupt)?;
            input
                .facts
                .capabilities
                .rebind_dependency(&previous, &digest);
            input.facts.dependency_digest = digest;
        }
        Ok(input)
    }

    fn skill_only_settings_snapshot(
        &self,
        spec: &AgentSettingsSpecV2,
        class: SettingsAgentClass,
    ) -> Result<AgentSettingsPlanningInput, ControlReadError> {
        let installation = if class == SettingsAgentClass::Hermes {
            hiroute_integrations::hermes_management_installation(
                &self.hermes_target_for_spec(spec)?,
            )
            .map_err(|_| ControlReadError::Denied)?
        } else if class.is_codex() || class == SettingsAgentClass::Claude {
            self.scanner
                .skill_settings_installation(class.agent_id())
                .ok_or(ControlReadError::NotFound)?
        } else {
            let discovery = match class {
                SettingsAgentClass::Pi => self.scanner.pi_settings_discovery(),
                SettingsAgentClass::Dsh => self.scanner.dsh_settings_discovery(),
                SettingsAgentClass::DshDesktop => self.scanner.dsh_desktop_settings_discovery(),
                SettingsAgentClass::OpenCode => self.scanner.opencode_settings_discovery(),
                _ => self.scanner.qoder_settings_discovery(true),
            };
            let AgentDiscoveryOutcomeV1::Supported { installation } = discovery.outcome else {
                return Err(ControlReadError::NotFound);
            };
            *installation
        };
        let subject = if class == SettingsAgentClass::Hermes {
            hiroute_integrations::hermes_subject(self.hermes_target_for_spec(spec)?.config_root())
                .map_err(|_| ControlReadError::Corrupt)?
        } else {
            AgentConnectionTransactionSubjectV1::from_registered_profile(
                installation.agent_id.clone(),
                installation.profile.profile_id.clone(),
                installation.profile.integration_profile_ref.clone(),
            )
            .map_err(|_| ControlReadError::Corrupt)?
        };
        let (file, conflict, restores) = self.capture_skill_resource(spec, spec.skill_facet())?;
        let revisions = self
            .stores_lock()
            .map_err(super::super::map_port)?
            .control()
            .current_revisions(&WorkspaceId::default())
            .map_err(super::super::map_port)?;
        let collaboration_state = self.settings_collaboration_state(spec)?;
        let semantic: Vec<_> = installation
            .capability_evidence
            .iter()
            .map(|e| {
                (
                    e.capability,
                    e.state,
                    &e.adapter_contract,
                    &e.dependency_digest,
                    e.reason,
                )
            })
            .collect();
        let digest = CanonicalDigest::of(&serde_json::json!({
            "schema":"agent-skill-only-facts/v1", "context":spec.context_id,
            "observation":installation.observation_digest,"capabilities":semantic,"revisions":revisions,
            "root":file.root_ref,"target":file.target,"before":file.before,"observed":file.observed_file,
            "fingerprint":file.before_fingerprint,"template":file.template.digest,"restores":restores,
            "collaboration":collaboration_state,"conflict":conflict
        })).map_err(|_| ControlReadError::Corrupt)?;
        let capabilities =
            AgentCapabilitySet::new(installation.capability_evidence.into_iter().map(|mut e| {
                if e.dependency_digest == installation.observation_digest {
                    e.dependency_digest = digest.clone();
                }
                e
            }))
            .map_err(|_| ControlReadError::Corrupt)?;
        Ok(AgentSettingsPlanningInput {
            additional_skill_file: None,
            skill_file: file,
            model_file: None,
            subject,
            expected_revisions: revisions,
            collaboration_state,
            facts: AgentSettingsFacts {
                reference_only_skill_restores: BTreeSet::new(),
                native_target: if class == SettingsAgentClass::Hermes {
                    Some(self.hermes_target_for_spec(spec)?)
                } else {
                    None
                },
                context_id: spec.context_id.clone(),
                dependency_digest: digest,
                capabilities,
                model: None,
                restore_points: restores,
                collaboration_file_conflict: spec.skill_facet()
                    == AgentSettingsFacet::Collaboration
                    && conflict,
                management_file_conflict: spec.skill_facet() == AgentSettingsFacet::ManagementSkill
                    && conflict,
            },
        })
    }

    fn capture_skill_resource(
        &self,
        spec: &AgentSettingsSpecV2,
        facet: AgentSettingsFacet,
    ) -> Result<
        (
            SettingsSkillFileFacts,
            bool,
            BTreeMap<String, AgentSettingsFacet>,
        ),
        ControlReadError,
    > {
        let (root_ref, target) =
            self.settings_skill_resource(&spec.context_id, spec.native_target.as_ref(), facet)?;
        let intent = spec.skill_intent_for(facet);
        let stores = self.stores_lock().map_err(super::super::map_port)?;
        let before = stores
            .control()
            .skill_installation(&WorkspaceId::default(), &root_ref)
            .map_err(super::super::map_port)?;
        let (fingerprint, observed) = if matches!(intent, AgentFacetIntent::Keep)
            || (matches!(intent, AgentFacetIntent::Restore { .. })
                && before.as_ref().is_some_and(|r| {
                    r.file_ownership
                        == hiroute_domain::CollaborationSkillFileOwnership::BorrowedIdentical
                })) {
            (None, None)
        } else {
            let fingerprint = self
                .artifacts
                .current_external_fingerprint(&target)
                .map_err(super::super::map_port)?;
            let bytes = self
                .artifacts
                .read_native_target(&target)
                .map_err(super::super::map_port)?;
            if fingerprint
                != self
                    .artifacts
                    .current_external_fingerprint(&target)
                    .map_err(super::super::map_port)?
            {
                return Err(ControlReadError::SnapshotChanged);
            }
            (fingerprint, bytes.map(|b| CanonicalDigest::of_bytes(&b)))
        };
        let mut restores = BTreeMap::new();
        if before
            .as_ref()
            .is_some_and(|r| r.contexts.contains(&spec.context_id))
        {
            for operation in stores
                .control()
                .succeeded_agent_operations_for_kind(
                    &WorkspaceId::default(),
                    "ApplyAgentConnectionChange",
                )
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
                if matches!(
                    original.skill_intent_for(facet),
                    AgentFacetIntent::Configure { .. }
                ) {
                    restores.insert(
                        skill_restore_ref(&spec.context_id, facet, &operation.operation_id),
                        facet,
                    );
                    if facet == spec.skill_facet() && matches!(spec.model, AgentFacetIntent::Keep) {
                        restores.insert(
                            codex_model_restore_point_ref(&operation.operation_id),
                            facet,
                        );
                    }
                    break;
                }
            }
        }
        drop(stores);
        let template = if facet == AgentSettingsFacet::ManagementSkill {
            super::hermes::management_template()?
        } else {
            collaboration_template(spec)?
        };
        let conflict = match intent {
            AgentFacetIntent::Keep => false,
            AgentFacetIntent::Configure { .. } => plan_skill_install(
                &root_ref,
                &spec.context_id,
                &template,
                before.as_ref(),
                observed.as_ref(),
            )
            .is_err(),
            AgentFacetIntent::Restore { .. } => before.as_ref().is_some_and(|r| {
                plan_skill_remove(&spec.context_id, r, observed.as_ref())
                    .map_or(true, |p| p.file_action == SkillFileAction::Conflict)
            }),
        };
        let releases_borrowed = matches!(intent, AgentFacetIntent::Restore { .. })
            && before.as_ref().is_some_and(|r| {
                r.file_ownership
                    == hiroute_domain::CollaborationSkillFileOwnership::BorrowedIdentical
            });
        let (observed, fingerprint) = if releases_borrowed {
            (None, None)
        } else {
            (observed, fingerprint)
        };
        Ok((
            SettingsSkillFileFacts {
                root_ref,
                target,
                before,
                observed_file: observed,
                before_fingerprint: fingerprint,
                template,
            },
            conflict,
            restores,
        ))
    }
}
