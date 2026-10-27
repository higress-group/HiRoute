//! The management Skill reports its own file ownership, never model or Worker authority.
use super::*;
use hiroute_application_api::{
    AgentManagementSkillStatusV1, AgentModelSettingsStateV2 as State, AgentSettingsStatusRequestV2,
};
use hiroute_domain::{
    AgentFacetIntent, AgentOperationRead, AgentSettingsSpecV2, NativeAgentArtifactPort,
    OperationState,
};

impl LocalControlAdapter {
    pub(super) fn skill_resource_status(
        &self,
        request: &AgentSettingsStatusRequestV2,
        facet: hiroute_domain::AgentSettingsFacet,
    ) -> Result<hiroute_application_api::AgentSkillResourceStatusV1, ControlReadError> {
        let (root_ref, target) = self.settings_skill_resource(
            &request.context_id,
            request.native_target.as_ref(),
            facet,
        )?;
        let path = self
            .artifacts
            .native_target_path(&target)
            .map_err(super::map_port)?;
        let record = self
            .stores_lock()
            .map_err(super::map_port)?
            .control()
            .skill_installation(&WorkspaceId::default(), &root_ref)
            .map_err(super::map_port)?;
        let native_available = self
            .artifacts
            .read_native_target(&target)
            .ok()
            .map(|bytes| {
                bytes.as_ref().is_some_and(|bytes| {
                    let digest = CanonicalDigest::of_bytes(bytes);
                    if let Some(record) = &record {
                        return digest == record.content_digest;
                    }
                    facet == hiroute_domain::AgentSettingsFacet::ManagementSkill
                        && digest
                            == CanonicalDigest::of_bytes(include_bytes!(
                                "../../../../../assets/skills/hiroute-management/SKILL.md"
                            ))
                })
            });
        Ok(hiroute_application_api::AgentSkillResourceStatusV1 {
            directory: path
                .parent()
                .and_then(|p| p.to_str())
                .ok_or(ControlReadError::Corrupt)?
                .to_owned(),
            file_ownership: record.as_ref().map(|r| r.file_ownership),
            shared_context_count: record.as_ref().map_or(0, |r| r.contexts.len()),
            native_available,
            runtime_loading: hiroute_application_api::AgentNativeRuntimeLoadingV1::Unobserved,
        })
    }

    pub(super) fn management_settings_status(
        &self,
        request: &AgentSettingsStatusRequestV2,
    ) -> Result<AgentManagementSkillStatusV1, ControlReadError> {
        let mut status = AgentManagementSkillStatusV1 {
            resource: None,
            schema: "hiroute.agent-management-skill-status/v1".into(),
            state: State::NotConfigured,
            operation_id: None,
            operation_state: None,
            restore_point_ref: None,
        };
        let (root_ref, file_target) = self.settings_skill_resource(
            &request.context_id,
            request.native_target.as_ref(),
            hiroute_domain::AgentSettingsFacet::ManagementSkill,
        )?;
        if request.include_skills {
            status.resource = Some(self.skill_resource_status(
                request,
                hiroute_domain::AgentSettingsFacet::ManagementSkill,
            )?);
        }
        let stores = self.stores_lock().map_err(super::map_port)?;
        let belongs = |operation: &dyn AgentOperationRead| -> Result<bool, ControlReadError> {
            let input = operation.agent_input();
            if input.spec.command_id != "agents.settings.apply"
                || input.spec.resource_id.as_deref() != Some(&request.context_id)
            {
                return Ok(false);
            }
            let spec: AgentSettingsSpecV2 =
                serde_json::from_value(input.spec.desired_state.clone())
                    .map_err(|_| ControlReadError::Corrupt)?;
            Ok(spec
                .management_skill
                .as_ref()
                .is_some_and(|i| !matches!(i, AgentFacetIntent::Keep)))
        };
        for pending in stores
            .control()
            .recoverable_operations()
            .map_err(super::map_port)?
        {
            if belongs(&pending)? {
                status.state = if pending.state == OperationState::NeedsAttention {
                    State::NeedsAttention
                } else {
                    State::Pending
                };
                status.operation_id = Some(pending.operation_id.to_string());
                status.operation_state = Some(pending.state.as_str().into());
                return Ok(status);
            }
        }
        let selected = stores
            .control()
            .find_verified_agent_operation(
                &WorkspaceId::default(),
                &["ApplyAgentConnectionChange", "ApplyAgentConnectionRestore"],
                |operation| {
                    belongs(operation).map_err(|_| {
                        hiroute_domain::PortError::new(
                            hiroute_domain::PortErrorCode::Corrupt,
                            "agent.management.current-fact",
                        )
                    })
                },
            )
            .map_err(super::map_port)?;
        let Some(operation) = selected else {
            return Ok(status);
        };
        let spec: AgentSettingsSpecV2 =
            serde_json::from_value(operation.plan.spec().desired_state.clone())
                .map_err(|_| ControlReadError::Corrupt)?;
        let record = stores
            .control()
            .skill_installation(&WorkspaceId::default(), &root_ref)
            .map_err(super::map_port)?;
        drop(stores);
        status.operation_id = Some(operation.operation_id.to_string());
        status.operation_state = Some(operation.state.as_str().into());
        match spec.management_skill {
            Some(AgentFacetIntent::Restore { .. }) => {
                status.state = if record.is_none_or(|r| !r.contexts.contains(&request.context_id)) {
                    State::Restored
                } else {
                    State::Drift
                }
            }
            Some(AgentFacetIntent::Configure { .. }) => {
                let bytes = self
                    .artifacts
                    .read_native_target(&file_target)
                    .ok()
                    .flatten();
                let current = record.is_some_and(|r| {
                    r.contexts.contains(&request.context_id)
                        && bytes
                            .as_ref()
                            .is_some_and(|b| CanonicalDigest::of_bytes(b) == r.content_digest)
                });
                status.state = if current {
                    State::Configured
                } else {
                    State::Drift
                };
                status.restore_point_ref = Some(if request.include_skills {
                    super::settings_facts::skills::skill_restore_ref(
                        &request.context_id,
                        hiroute_domain::AgentSettingsFacet::ManagementSkill,
                        &operation.operation_id,
                    )
                } else {
                    hiroute_application::agent_connection::codex_model_restore_point_ref(
                        &operation.operation_id,
                    )
                });
            }
            _ => return Err(ControlReadError::Corrupt),
        }
        Ok(status)
    }
}
