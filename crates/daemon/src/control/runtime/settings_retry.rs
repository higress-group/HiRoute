//! Resume only the original context-bound native settings Operation and its sealed effects.
use super::settings_facts::SettingsAgentClass;
use super::*;
use hiroute_domain::{AgentFacetIntent, AgentSettingsSpecV2, ControlRepositoryPort};

impl LocalControlAdapter {
    pub(super) fn retry_model_settings_operation(
        &self,
        request: &hiroute_application_api::AgentSettingsRetryV1,
    ) -> Result<hiroute_domain::OperationV1, ControlReadError> {
        if request.schema != "hiroute.agent-settings-retry/v1"
            || self
                .settings_agent_for_context(&request.context_id)
                .is_none_or(|class| !class.is_codex() && class != SettingsAgentClass::Qoder)
        {
            return Err(ControlReadError::Denied);
        }
        let stores = self.stores_lock().map_err(super::map_port)?;
        let op = stores
            .control()
            .load_operation(&request.operation_id)
            .map_err(super::map_port)?
            .ok_or(ControlReadError::NotFound)?;
        if op.workspace_id != WorkspaceId::default()
            || op.plan.spec().command_id != "agents.settings.apply"
            || op.plan.spec().resource_id.as_deref() != Some(&request.context_id)
        {
            return Err(ControlReadError::Denied);
        }
        let spec: AgentSettingsSpecV2 =
            serde_json::from_value(op.plan.spec().desired_state.clone())
                .map_err(|_| ControlReadError::Corrupt)?;
        if matches!(spec.model, AgentFacetIntent::Keep) {
            return Err(ControlReadError::Denied);
        }
        if self.settings_agent_for_context(&request.context_id) == Some(SettingsAgentClass::Qoder) {
            let intent = op
                .plan
                .external()
                .iter()
                .find(|intent| super::native_qoder_model::is_settings_qoder_model(intent))
                .ok_or(ControlReadError::Denied)?;
            hiroute_application::agent_connection::settings_qoder_model_file_for_operation(
                &op, intent,
            )
            .map_err(super::map_port)?;
        }
        drop(stores);
        hiroute_application::TransactionCoordinator::new(
            self,
            self,
            self,
            self,
            self,
            &self.admission,
        )
        .run(&request.operation_id)
        .map_err(|_| ControlReadError::Unavailable)
    }
}
