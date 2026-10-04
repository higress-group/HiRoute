use hiroute_domain::delegation::DelegationErrorV1;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "mapping", rename_all = "snake_case", deny_unknown_fields)]
pub enum AcpNativeIdentityContract {
    #[default]
    Unverified,
    ExplicitResponseMetadata,
    CodexThreadV1,
    ClaudeSessionV1,
    QoderSessionV1,
}

impl AcpNativeIdentityContract {
    pub(super) fn new_native(
        &self,
        acp_id: &str,
        response: &Value,
    ) -> Result<Option<String>, DelegationErrorV1> {
        match self {
            Self::Unverified => Ok(None),
            Self::ExplicitResponseMetadata => Ok(metadata_id(response)),
            Self::CodexThreadV1 | Self::ClaudeSessionV1 | Self::QoderSessionV1 => {
                if !valid_id(acp_id) {
                    return Err(DelegationErrorV1::ProtocolFailed);
                }
                Ok(Some(acp_id.to_owned()))
            }
        }
    }

    pub(super) fn verify_load(&self, binding: &super::AcpSessionBinding, response: &Value) -> bool {
        match self {
            Self::Unverified => false,
            Self::ExplicitResponseMetadata => {
                metadata_id(response).as_ref() == binding.native_session_id.as_ref()
            }
            Self::CodexThreadV1 | Self::ClaudeSessionV1 | Self::QoderSessionV1 => {
                valid_id(&binding.acp_session_id)
                    && binding.native_session_id.as_deref() == Some(binding.acp_session_id.as_str())
            }
        }
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 1024 && !id.chars().any(char::is_control)
}

fn metadata_id(response: &Value) -> Option<String> {
    response
        .pointer("/_meta/agentSessionId")
        .or_else(|| response.pointer("/_meta/sessionId"))
        .and_then(Value::as_str)
        .filter(|id| valid_id(id))
        .map(str::to_owned)
}
