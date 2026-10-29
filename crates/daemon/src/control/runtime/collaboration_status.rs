//! Current non-secret collaboration projection for the native Desktop task surface.

use super::LocalControlAdapter;
use super::settings_facet_read::FacetSelection;
use hiroute_application::control::ControlReadError;
use hiroute_application_api::{
    AgentCollaborationSettingsStatusV2, AgentModelSettingsStateV2 as State,
    AgentSettingsStatusRequestV2,
};
use hiroute_diagnostics::agent_read::{
    AgentReadCategory as Category, AgentReadStage as Stage, Timer,
};
use hiroute_domain::{
    AgentFacetIntent, AgentOperationRead, AgentSettingsSpecV2, CanonicalDigest,
    NativeAgentArtifactPort, OperationState,
};

impl LocalControlAdapter {
    pub(super) fn collaboration_settings_status(
        &self,
        request: &AgentSettingsStatusRequestV2,
    ) -> Result<AgentCollaborationSettingsStatusV2, ControlReadError> {
        let diagnostics = self.agent_read_diagnostics();
        let _total = Timer::start(&diagnostics, Category::Collaboration, Stage::Total);
        match self.collaboration_settings_status_once(request) {
            Err(ControlReadError::SnapshotChanged) => {
                self.collaboration_settings_status_once(request)
            }
            result => result,
        }
    }

    fn collaboration_settings_status_once(
        &self,
        request: &AgentSettingsStatusRequestV2,
    ) -> Result<AgentCollaborationSettingsStatusV2, ControlReadError> {
        let read = self
            .capture_settings_facet(request, hiroute_domain::AgentSettingsFacet::Collaboration)?;
        let diagnostics = self.agent_read_diagnostics();
        let native = Timer::start(&diagnostics, Category::Collaboration, Stage::NativeRead);
        let mut status = empty_status();
        if request.include_skills {
            status.resource = Some(self.skill_resource_status(
                hiroute_domain::AgentSettingsFacet::Collaboration,
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
            serde_json::from_value(operation.agent_input().spec.desired_state.clone())
                .map_err(|_| ControlReadError::Corrupt)?;
        status.operation_id = Some(operation.operation_id.to_string());
        status.operation_state = Some(operation.state.as_str().into());
        let skill = read.skill.as_ref().ok_or(ControlReadError::Corrupt)?;
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
                let skill_current = match skill.as_ref() {
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
                                .read_native_target(&read.target)
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
        drop(native);
        self.fence_settings_facet(&read)?;
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
