//! One current live-check result per settings context and client surface.
use crate::{AgentModelSurfaceV2, CanonicalDigest, GatewayPublicationRevision};
use serde::{Deserialize, Serialize};

pub const AGENT_SURFACE_CHECK_SCHEMA: &str = "hiroute.agent-surface-check/v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("agent surface check record is invalid")]
pub struct AgentSurfaceCheckValidationError;

/// The durable current result of one trusted executor's live model check for one surface.
/// `not_verified` is never stored: it is derived by the status join when no record matches
/// the currently installed publication revision.
#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSurfaceCheckStateV1 {
    Passed,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSurfaceCheckRecordV1 {
    pub schema: String,
    pub context_id: String,
    pub surface: AgentModelSurfaceV2,
    pub applied_revision: GatewayPublicationRevision,
    pub state: AgentSurfaceCheckStateV1,
    pub checked_model_ids: Vec<String>,
    /// Digest of the verified client capability and account scope evidence.
    pub capability_scope_digest: CanonicalDigest,
    /// Digest of the original AgentCheckRequestV1 this result answers.
    pub check_request_digest: CanonicalDigest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
}

impl AgentSurfaceCheckRecordV1 {
    pub fn validate(&self) -> Result<(), AgentSurfaceCheckValidationError> {
        if self.schema != AGENT_SURFACE_CHECK_SCHEMA
            || self.context_id.is_empty()
            || self.context_id.len() > 256
            || self.applied_revision.get() == 0
            || self.checked_model_ids.is_empty()
            || self.checked_model_ids.len() > 128
            || self.checked_model_ids.iter().any(|name| {
                name.is_empty() || name.bytes().any(|byte| matches!(byte, 0 | b'\r' | b'\n'))
            })
            || self
                .reason_code
                .as_ref()
                .is_some_and(|code| code.len() > 128)
        {
            return Err(AgentSurfaceCheckValidationError);
        }
        Ok(())
    }
}
