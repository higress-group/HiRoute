//! Seal independently confirmed facets for the existing Operation engine.
use super::ConfirmedAgentSettings;
use hiroute_domain::{
    AgentAccessGrantMutationV1, AgentConnectionControlIntentV1, AgentConnectionTransactionKindV1,
    AgentConnectionTransactionSubjectV1, CHANGE_SPEC_SCHEMA_V1, ChangeSpecV1,
    ExternalEffectIntentV1, OperationValidationError, TransactionPlanV1,
};
use serde::Serialize;

impl ConfirmedAgentSettings {
    /// Pure sealing, not admission or execution. The registered handler must still reproduce
    /// facts inside the existing writer boundary and consume the protected Apply capability.
    /// `state` contains backend-owned non-secret mutations/recovery references; protected native
    /// bytes and credentials stay in the existing stores. The renderer binds each exact file
    /// effect to this control intent, including the shared Skill file/no-file decision.
    pub fn seal_effects<T: Serialize>(
        &self,
        subject: AgentConnectionTransactionSubjectV1,
        state: &T,
        skill_file_change: bool,
        model_grants: Vec<AgentAccessGrantMutationV1>,
        render: impl FnOnce(
            &AgentConnectionControlIntentV1,
        ) -> Result<Vec<ExternalEffectIntentV1>, OperationValidationError>,
    ) -> Result<TransactionPlanV1, OperationValidationError> {
        self.seal_effects_with_skills(
            subject,
            state,
            skill_file_change,
            false,
            model_grants,
            render,
        )
    }

    pub fn seal_effects_with_skills<T: Serialize>(
        &self,
        subject: AgentConnectionTransactionSubjectV1,
        state: &T,
        skill_file_change: bool,
        additional_skill_file_change: bool,
        model_grants: Vec<AgentAccessGrantMutationV1>,
        render: impl FnOnce(
            &AgentConnectionControlIntentV1,
        ) -> Result<Vec<ExternalEffectIntentV1>, OperationValidationError>,
    ) -> Result<TransactionPlanV1, OperationValidationError> {
        let preview = self.preview();
        let spec = ChangeSpecV1 {
            schema_version: CHANGE_SPEC_SCHEMA_V1,
            command_id: AgentConnectionTransactionKindV1::Settings
                .command_id()
                .into(),
            resource_id: Some(preview.spec.context_id.clone()),
            desired_state: serde_json::to_value(&preview.spec)?,
        };
        let control = AgentConnectionControlIntentV1::from_settings_planner_with_skills(
            subject,
            &spec,
            skill_file_change,
            additional_skill_file_change,
            &serde_json::json!({
                "accept_digest": preview.accept_digest,
                "dependency_digest": preview.dependency_digest,
                "model_grant": preview.model_grant,
                "collaboration_trigger_mode": preview.collaboration_trigger_mode,
                "mutations": state,
            }),
        )?;
        let external = render(&control)?;
        TransactionPlanV1::from_agent_connection_planner_with_agent_access_grants(
            spec,
            control,
            model_grants,
            external,
        )
    }
}
