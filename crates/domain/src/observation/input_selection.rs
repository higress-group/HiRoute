//! Request-owned excerpt position; matching is not proof of human authorship.
use serde::{Deserialize, Serialize};

use super::ExecutionFactError;

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestInputSelectionEvidenceV1 {
    NativeMatched,
    WireObserved,
    Unavailable,
}

/// The first message of the selected input, using original capture ordinals.
/// A batch excerpt does not claim to reproduce the complete input batch.
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RequestInputSelectionV1 {
    pub version: u8,
    pub evidence: RequestInputSelectionEvidenceV1,
    pub message_ordinal: Option<u32>,
    pub source_unverified: bool,
}

impl RequestInputSelectionV1 {
    pub fn validate(&self) -> Result<(), ExecutionFactError> {
        let unavailable = self.evidence == RequestInputSelectionEvidenceV1::Unavailable;
        if self.version != 1
            || self.message_ordinal.is_none() != unavailable
            || self.message_ordinal.is_some_and(|ordinal| ordinal > 65_535)
        {
            return Err(ExecutionFactError::InvalidFact);
        }
        Ok(())
    }
}
