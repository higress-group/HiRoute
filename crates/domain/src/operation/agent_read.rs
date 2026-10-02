//! Borrowed business facts for Agent status. This view cannot enter the transaction executor.
use super::*;

pub struct AgentOperationInputView<'a> {
    pub spec: &'a ChangeSpecV1,
    pub control: &'a Value,
    pub external: &'a [ExternalEffectIntentV1],
    pub grants: &'a [AgentAccessGrantMutationV1],
}

pub trait AgentOperationRead {
    fn agent_input(&self) -> AgentOperationInputView<'_>;
    fn operation_id(&self) -> &OperationId;
    fn accepted_digest(&self) -> &CanonicalDigest;
    fn state(&self) -> OperationState;
}

impl AgentOperationRead for OperationV1 {
    fn agent_input(&self) -> AgentOperationInputView<'_> {
        AgentOperationInputView {
            spec: self.plan.spec(),
            control: self.plan.control(),
            external: self.plan.external(),
            grants: self.plan.agent_access_grants(),
        }
    }
    fn operation_id(&self) -> &OperationId {
        &self.operation_id
    }
    fn accepted_digest(&self) -> &CanonicalDigest {
        &self.accepted_digest
    }
    fn state(&self) -> OperationState {
        self.state
    }
}

/// Validated command facts, with no routing compiler, journal writer or execution plan.
#[derive(Clone, Debug)]
pub struct AgentPlanReadV1 {
    spec: ChangeSpecV1,
    control: Value,
    external: Vec<ExternalEffectIntentV1>,
    grants: Vec<AgentAccessGrantMutationV1>,
}

impl AgentPlanReadV1 {
    pub fn from_stored(input: &StoredOperationInputV1) -> Result<Self, OperationValidationError> {
        if input.schema != OPERATION_INPUT_SCHEMA
            || input.credential_pool.is_some()
            || input.worker_dependency_selection.is_some()
            || !input.secrets.is_empty()
            || !input.runtime.is_empty()
        {
            return Err(OperationValidationError::UnregisteredEffectPlan);
        }
        let external = input
            .external
            .iter()
            .map(|intent| {
                ExternalEffectIntentV1::from_registered_adapter(
                    intent.effect_id.clone(),
                    intent.kind,
                    intent.target.clone(),
                    intent.before_fingerprint.clone(),
                    intent.desired.clone(),
                    intent.desired_mode,
                    intent.sensitive,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        agent_connection::validate_plan(&input.spec, &input.control, &[], &[], &external)?;
        let grants = agent_connection::agent_access_grants_from_control(&input.control)?;
        Ok(Self {
            spec: input.spec.clone(),
            control: input.control.clone(),
            external,
            grants,
        })
    }
    pub fn spec(&self) -> &ChangeSpecV1 {
        &self.spec
    }
    pub fn operation_kind(&self) -> Result<&'static str, OperationValidationError> {
        match self.spec.command_id.as_str() {
            "agents.connect.apply" => Ok("ApplyAgentConnectionChange"),
            "agents.restore.apply" => Ok("ApplyAgentConnectionRestore"),
            "agents.settings.apply" => {
                let settings: crate::AgentSettingsSpecV2 =
                    serde_json::from_value(self.spec.desired_state.clone())
                        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
                Ok(if settings.is_restore_only() {
                    "ApplyAgentConnectionRestore"
                } else {
                    "ApplyAgentConnectionChange"
                })
            }
            _ => Err(OperationValidationError::UnregisteredEffectPlan),
        }
    }
    pub fn control(&self) -> &Value {
        &self.control
    }
    pub fn external(&self) -> &[ExternalEffectIntentV1] {
        &self.external
    }
    pub fn agent_access_grants(&self) -> &[AgentAccessGrantMutationV1] {
        &self.grants
    }
    pub fn agent_connection_projection(
        &self,
    ) -> Result<Option<ActiveAgentConnectionV1>, OperationValidationError> {
        agent_connection::connection_projection(&self.spec, &self.control)
    }
}
