use serde::{Deserialize, Serialize};

use super::{AgentTurnOpeningCauseV1, ExecutionFactError, TurnId};

/// The Gateway's request-time admission identity, independent of a routing
/// decision or a later stage closure. The envelope supplies request identity
/// and producer epoch. This does not prove human origin, history association,
/// execution completeness, a segment identity, or logical task completion.
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentTurnAdmissionV1 {
    pub version: u32,
    pub agent_turn_id: String,
    pub branch_id: String,
    pub revision: u64,
    pub ordinal: u64,
    pub opening_cause: AgentTurnOpeningCauseV1,
}

impl AgentTurnAdmissionV1 {
    pub fn validate(&self) -> Result<(), ExecutionFactError> {
        if self.version != 1
            || TurnId::parse(&self.agent_turn_id).is_err()
            || self.branch_id.trim().is_empty()
            || self.branch_id.len() > 128
            || self.branch_id.chars().any(char::is_control)
            || self.revision == 0
            || self.ordinal == 0
        {
            return Err(ExecutionFactError::InvalidFact);
        }
        Ok(())
    }
}
