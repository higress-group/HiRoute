//! Read-only failure evidence in the existing step result, never execution authority.
use super::OperationStepKind;
use crate::{PortError, PortErrorCode};
use serde::{Deserialize, Serialize};

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationFailureCauseV1 {
    Conflict,
    Corrupt,
    Crypto,
    InvalidData,
    NotFound,
    PermissionDenied,
    Unavailable,
    Rejected,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OperationFailureV1 {
    pub schema: String,
    pub step: OperationStepKind,
    pub code: String,
    pub cause: OperationFailureCauseV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter_context: Option<String>,
}

impl OperationFailureV1 {
    pub fn new(step: OperationStepKind, code: &str, port: Option<&PortError>) -> Self {
        let cause = port.map_or(OperationFailureCauseV1::Rejected, |port| match port.code {
            PortErrorCode::Conflict => OperationFailureCauseV1::Conflict,
            PortErrorCode::Corrupt => OperationFailureCauseV1::Corrupt,
            PortErrorCode::Crypto => OperationFailureCauseV1::Crypto,
            PortErrorCode::InvalidData => OperationFailureCauseV1::InvalidData,
            PortErrorCode::NotFound => OperationFailureCauseV1::NotFound,
            PortErrorCode::PermissionDenied => OperationFailureCauseV1::PermissionDenied,
            PortErrorCode::Unavailable => OperationFailureCauseV1::Unavailable,
        });
        Self {
            schema: "hiroute.operation-failure/v1".into(),
            step,
            code: code.into(),
            cause,
            adapter_context: port
                .map(|port| port.context)
                .filter(|context| safe_context(context))
                .map(str::to_owned),
        }
    }

    pub fn parse(result: &str) -> Option<Self> {
        if result.len() > 512 {
            return None;
        }
        let value: Self = serde_json::from_str(result).ok()?;
        (value.schema == "hiroute.operation-failure/v1"
            && !value.code.is_empty()
            && value.code.len() <= 64
            && value
                .code
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b == b'_')
            && value.adapter_context.as_deref().is_none_or(safe_context))
        .then_some(value)
    }
}

fn safe_context(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_evidence_rejects_raw_paths_and_unrecognized_results() {
        let error = PortError::new(PortErrorCode::PermissionDenied, "C:\\Users\\private\\token");
        let value = OperationFailureV1::new(
            OperationStepKind::ApplyAgentArtifacts,
            "ADAPTER_FAILURE",
            Some(&error),
        );
        assert!(value.adapter_context.is_none());
        assert_eq!(
            OperationFailureV1::parse(&serde_json::to_string(&value).unwrap()),
            Some(value)
        );
        assert!(OperationFailureV1::parse("compensated").is_none());
        assert!(OperationFailureV1::parse(r#"{"schema":"unknown","step":"activate","code":"ADAPTER_FAILURE","cause":"unavailable"}"#).is_none());
    }
}
