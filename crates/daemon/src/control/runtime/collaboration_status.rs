//! Current non-secret collaboration projection for the native Desktop task surface.

use super::LocalControlAdapter;
use hiroute_application::control::ControlReadError;
use hiroute_application_api::{
    AgentCollaborationSettingsStatusV2, AgentModelSettingsStateV2 as State,
    AgentSettingsStatusRequestV2,
};
use hiroute_domain::{
    AgentFacetIntent, AgentOperationRead, AgentSettingsSpecV2, CanonicalDigest,
    NativeAgentArtifactPort, OperationState, WorkspaceId,
};

impl LocalControlAdapter {
    pub(super) fn collaboration_settings_status(
        &self,
        request: &AgentSettingsStatusRequestV2,
    ) -> Result<AgentCollaborationSettingsStatusV2, ControlReadError> {
        let (root_ref, target) = self.settings_skill_resource(
            &request.context_id,
            request.native_target.as_ref(),
            hiroute_domain::AgentSettingsFacet::Collaboration,
        )?;
        let mut status = empty_status();
        if request.include_skills {
            status.resource = Some(self.skill_resource_status(
                request,
                hiroute_domain::AgentSettingsFacet::Collaboration,
            )?);
        }
        let stores = self.stores_lock().map_err(super::map_port)?;
        let belongs =
            |operation: &dyn hiroute_domain::AgentOperationRead| -> Result<bool, ControlReadError> {
                if operation.agent_input().spec.command_id != "agents.settings.apply"
                    || operation.agent_input().spec.resource_id.as_deref()
                        != Some(&request.context_id)
                {
                    return Ok(false);
                }
                let spec: AgentSettingsSpecV2 =
                    serde_json::from_value(operation.agent_input().spec.desired_state.clone())
                        .map_err(|_| ControlReadError::Corrupt)?;
                Ok(!matches!(spec.collaboration, AgentFacetIntent::Keep))
            };
        if let Some(pending) = stores
            .control()
            .writer_claim_operation()
            .map_err(super::map_port)?
            && belongs(&pending)?
        {
            status.state = if pending.state == OperationState::NeedsAttention {
                State::NeedsAttention
            } else {
                State::Pending
            };
            status.operation_id = Some(pending.operation_id.to_string());
            status.operation_state = Some(pending.state.as_str().into());
            return Ok(status);
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
                            "agent.collaboration.current-fact",
                        )
                    })
                },
            )
            .map_err(super::map_port)?;
        let Some(operation) = selected else {
            return Ok(status);
        };
        let spec: AgentSettingsSpecV2 =
            serde_json::from_value(operation.agent_input().spec.desired_state.clone())
                .map_err(|_| ControlReadError::Corrupt)?;
        status.operation_id = Some(operation.operation_id.to_string());
        status.operation_state = Some(operation.state.as_str().into());
        let skill = stores
            .control()
            .skill_installation(&operation.workspace_id, &root_ref)
            .map_err(super::map_port)?;
        drop(stores);
        match &spec.collaboration {
            AgentFacetIntent::Restore { .. } => {
                status.state = if skill
                    .as_ref()
                    .is_none_or(|record| !record.contexts.contains(&spec.context_id))
                {
                    State::Restored
                } else {
                    State::Drift
                };
            }
            AgentFacetIntent::Configure { settings } => {
                let skill_current = match skill {
                    Some(record) if record.contexts.contains(&spec.context_id) => {
                        let ownership_current = match record.file_ownership {
                            hiroute_domain::CollaborationSkillFileOwnership::Managed => {
                                record.file_effect.is_some()
                            }
                            hiroute_domain::CollaborationSkillFileOwnership::BorrowedIdentical => {
                                record.file_effect.is_none()
                            }
                        };
                        ownership_current
                            && self
                                .artifacts
                                .read_native_target(&target)
                                .ok()
                                .flatten()
                                .is_some_and(|bytes| {
                                    CanonicalDigest::of_bytes(bytes.as_slice())
                                        == record.content_digest
                                })
                    }
                    _ => false,
                };
                let configured = skill_current;
                status.state = if configured {
                    State::Configured
                } else {
                    State::Drift
                };
                status.restore_point_ref = Some(if request.include_skills {
                    super::settings_facts::skills::skill_restore_ref(
                        &request.context_id,
                        hiroute_domain::AgentSettingsFacet::Collaboration,
                        &operation.operation_id,
                    )
                } else {
                    hiroute_application::agent_connection::codex_model_restore_point_ref(
                        &operation.operation_id,
                    )
                });
                status.current_selection = configured.then_some(
                    hiroute_application_api::AgentCollaborationCurrentSelectionV2 {
                        trigger_mode: settings.trigger_mode,
                    },
                );
            }
            AgentFacetIntent::Keep => unreachable!("filtered above"),
        }
        Ok(status)
    }
}

fn empty_status() -> AgentCollaborationSettingsStatusV2 {
    AgentCollaborationSettingsStatusV2 {
        resource: None,
        schema: "hiroute.agent-collaboration-settings-status/v2".into(),
        state: State::NotConfigured,
        operation_id: None,
        operation_state: None,
        restore_point_ref: None,
        current_selection: None,
    }
}
