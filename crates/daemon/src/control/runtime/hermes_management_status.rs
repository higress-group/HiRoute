//! The management Skill reports its own file ownership, never model or Worker authority.
use super::settings_facet_read::FacetSelection;
use super::*;
use hiroute_application_api::{
    AgentManagementSkillStatusV1, AgentModelSettingsStateV2 as State, AgentSettingsStatusRequestV2,
};
use hiroute_diagnostics::agent_read::{
    AgentReadCategory as Category, AgentReadStage as Stage, Timer,
};
use hiroute_domain::{
    AgentFacetIntent, AgentSettingsSpecV2, NativeAgentArtifactPort, OperationState,
};

impl LocalControlAdapter {
    pub(super) fn skill_resource_status(
        &self,
        facet: hiroute_domain::AgentSettingsFacet,
        target: &str,
        record: Option<&hiroute_domain::ManagedCollaborationSkill>,
    ) -> Result<hiroute_application_api::AgentSkillResourceStatusV1, ControlReadError> {
        let path = self
            .artifacts
            .native_target_path(target)
            .map_err(super::map_port)?;
        let native_available = self.artifacts.read_native_target(target).ok().map(|bytes| {
            bytes.as_ref().is_some_and(|bytes| {
                let digest = CanonicalDigest::of_bytes(bytes);
                if let Some(record) = record {
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
            file_ownership: record.map(|r| r.file_ownership),
            shared_context_count: record.map_or(0, |r| r.contexts.len()),
            native_available,
            runtime_loading: hiroute_application_api::AgentNativeRuntimeLoadingV1::Unobserved,
        })
    }

    pub(super) fn management_settings_status(
        &self,
        request: &AgentSettingsStatusRequestV2,
    ) -> Result<AgentManagementSkillStatusV1, ControlReadError> {
        let diagnostics = self.agent_read_diagnostics();
        let _total = Timer::start(&diagnostics, Category::Management, Stage::Total);
        match self.management_settings_status_once(request) {
            Err(ControlReadError::SnapshotChanged) => self.management_settings_status_once(request),
            result => result,
        }
    }

    fn management_settings_status_once(
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
        let read = self
            .capture_settings_facet(request, hiroute_domain::AgentSettingsFacet::ManagementSkill)?;
        let diagnostics = self.agent_read_diagnostics();
        let native = Timer::start(&diagnostics, Category::Management, Stage::NativeRead);
        if request.include_skills {
            status.resource = Some(self.skill_resource_status(
                hiroute_domain::AgentSettingsFacet::ManagementSkill,
                &read.target,
                read.skill.as_ref().and_then(Option::as_ref),
            )?);
        }
        let operation = match &read.selection {
            FacetSelection::Pending(pending) => {
                status.state = if pending.state == OperationState::NeedsAttention {
                    State::NeedsAttention
                } else {
                    State::Pending
                };
                status.operation_id = Some(pending.operation_id.to_string());
                status.operation_state = Some(pending.state.as_str().into());
                drop(native);
                self.fence_settings_facet(&read)?;
                return Ok(status);
            }
            FacetSelection::Absent => {
                drop(native);
                self.fence_settings_facet(&read)?;
                return Ok(status);
            }
            FacetSelection::Found(operation) => operation,
        };
        let spec: AgentSettingsSpecV2 =
            serde_json::from_value(operation.plan.spec().desired_state.clone())
                .map_err(|_| ControlReadError::Corrupt)?;
        let record = read.skill.as_ref().ok_or(ControlReadError::Corrupt)?;
        status.operation_id = Some(operation.operation_id.to_string());
        status.operation_state = Some(operation.state.as_str().into());
        match spec.management_skill {
            Some(AgentFacetIntent::Restore { .. }) => {
                status.state = if record
                    .as_ref()
                    .is_none_or(|r| !r.contexts.contains(&request.context_id))
                {
                    State::Restored
                } else {
                    State::Drift
                }
            }
            Some(AgentFacetIntent::Configure { .. }) => {
                let bytes = self
                    .artifacts
                    .read_native_target(&read.target)
                    .ok()
                    .flatten();
                let current = record.as_ref().is_some_and(|r| {
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
        drop(native);
        self.fence_settings_facet(&read)?;
        Ok(status)
    }
}
