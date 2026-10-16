//! Existing management read results in the public contract owner.
use crate::*;
use hiroute_domain::{AgentSurfaceCheckStateV1, DecisionServiceV1, GatewayPublicationRevision};
use serde::Serialize;

#[derive(schemars::JsonSchema, Serialize)]
pub struct CliPreparedPreviewV1<T> {
    pub schema: String,
    pub preview: T,
    pub submission: CliPreparedSubmissionV1,
}
#[derive(schemars::JsonSchema, Serialize)]
pub struct CliPreparedSubmissionV1 {
    pub apply_file: String,
    pub command_id: String,
    pub arguments: Vec<String>,
    pub operation_lookup: OperationIdempotencyLookupV1,
    pub state: String,
}
#[derive(schemars::JsonSchema, Serialize)]
pub struct CliRequiredChoicesV1 {
    pub schema: String,
    pub required_choices: Vec<String>,
    pub available: serde_json::Value,
}

#[derive(schemars::JsonSchema, Serialize)]
pub struct DecisionServicesListV1 {
    pub services: Vec<DecisionServiceV1>,
}

#[derive(schemars::JsonSchema, Serialize)]
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum AgentCheckResultV1 {
    Configuration {
        schema: String,
        suite: AgentCheckSuiteV1,
        agent: Box<DiscoveredAgentV1>,
        model_call: bool,
        gateway_listener: String,
    },
    NativeAuthentication {
        schema: String,
        agent_id: String,
        native_authentication: String,
        model_call: bool,
        model_verified: bool,
    },
    Collaboration {
        schema: String,
        agent_id: String,
        skill_loading: String,
        trusted_cli_execution: String,
        model_call: bool,
    },
    Live {
        schema: String,
        agent_id: String,
        surface: AgentModelSurfaceV2,
        applied_revision: GatewayPublicationRevision,
        checked_model_ids: Vec<String>,
        requested_call_count: usize,
        call_count: usize,
        state: AgentSurfaceCheckStateV1,
        reason_code: Option<String>,
        model_call: bool,
    },
}

/// Included only when the CLI caller explicitly requests detailed input errors.
#[derive(schemars::JsonSchema, Clone, Serialize)]
pub struct CliInputDiagnosticV1 {
    #[schemars(length(max = 64))]
    pub category: String,
    #[schemars(length(max = 256))]
    pub path: String,
    #[schemars(length(max = 128))]
    pub expected: String,
}
#[derive(schemars::JsonSchema, Serialize)]
pub struct CliInputDiagnosticDataV1 {
    pub schema: String,
    pub command_id: String,
    pub diagnostic: CliInputDiagnosticV1,
}
