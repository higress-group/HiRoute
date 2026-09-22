use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PRODUCTION_EVIDENCE: &str = "installed-standalone-headless-management-loop";
pub const PRODUCTION_EVIDENCE_OWNER: &str = "TASK-142003";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioState {
    Green,
    ExpectedRed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentScenarioStateV1 {
    pub scenario_id: String,
    pub state: ScenarioState,
    pub evidence_owner: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocker: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentConnectionContractV1 {
    pub schema: String,
    pub scenario_id: String,
    pub process: String,
    pub proofs: BTreeSet<String>,
    pub scenario_states: Vec<AgentScenarioStateV1>,
}

impl AgentConnectionContractV1 {
    pub fn validate(&self) -> Result<(), AgentConnectionContractError> {
        if self.schema != "hiroute.product-e2e.agent-connection/v1"
            || self.scenario_id != "process-25005-agent-connection"
            || self.process != "PROCESS-25005"
            || self.proofs
                != BTreeSet::from([
                    "all_published_selected_scope".to_owned(),
                    "built_in_codex_claude_emulator".to_owned(),
                    "dynamic_catalog_static_fallback".to_owned(),
                    "exact_profile_config_precedence".to_owned(),
                    "field_owned_restore".to_owned(),
                    "grant_catalog_overlay_exact".to_owned(),
                    PRODUCTION_EVIDENCE.to_owned(),
                    "optional_exact_marker_rewrite".to_owned(),
                    "client_version_diagnostic_only".to_owned(),
                ])
        {
            return Err(AgentConnectionContractError::InvalidContract);
        }
        for scenario_id in [
            "profile-grant-catalog-contract",
            "adapter-emulator-contract",
            "field-owned-restore-contract",
        ] {
            let state = self
                .scenario_states
                .iter()
                .find(|value| value.scenario_id == scenario_id)
                .ok_or(AgentConnectionContractError::MissingInternalState)?;
            if state.state != ScenarioState::Green
                || state.evidence_owner != "PROCESS-25005"
                || state.blocker.is_some()
            {
                return Err(AgentConnectionContractError::InvalidInternalState);
            }
        }
        let public = self
            .scenario_states
            .iter()
            .find(|value| value.scenario_id == "production-cli-agent-connection")
            .ok_or(AgentConnectionContractError::MissingPublicState)?;
        if public.state != ScenarioState::Green
            || public.evidence_owner != PRODUCTION_EVIDENCE_OWNER
            || public.blocker.is_some()
        {
            return Err(AgentConnectionContractError::InvalidPublicState);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum AgentConnectionContractError {
    #[error("AgentConnection Product E2E contract is invalid")]
    InvalidContract,
    #[error("an internal AgentConnection contract state is missing")]
    MissingInternalState,
    #[error("an internal AgentConnection contract state is not green")]
    InvalidInternalState,
    #[error("the public AgentConnection subprocess state is missing")]
    MissingPublicState,
    #[error("the public AgentConnection subprocess is not green")]
    InvalidPublicState,
}
