//! Frozen native decision input policy. This is independent of C question versions.
use serde::{Deserialize, Serialize};

use super::DecisionWireProtocolV2;
use crate::CanonicalDigest;

pub const DECISION_CONTEXT_FALLBACK_TOKENS: u32 = 32_000;
pub const DECISION_CONTEXT_ALGORITHM_V1: &str = "escaped-utf8-v1";
pub const DECISION_CONTEXT_RESERVE_V1: &str = "five-percent-min-1024-v1";

#[derive(
    schemars::JsonSchema, Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum DecisionContextModeV1 {
    #[default]
    Automatic,
    Custom,
}

#[derive(
    schemars::JsonSchema, Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum TypeSafeLongContextV1 {
    #[default]
    Automatic,
    Disabled,
}

#[derive(schemars::JsonSchema, Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionContextPolicyV1 {
    pub mode: DecisionContextModeV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_question_tokens: Option<u32>,
    pub typesafe_long_context: TypeSafeLongContextV1,
}

impl DecisionContextPolicyV1 {
    pub fn validate(&self) -> bool {
        match self.mode {
            DecisionContextModeV1::Automatic => {
                self.window_tokens.is_none() && self.per_question_tokens.is_none()
            }
            DecisionContextModeV1::Custom => self.window_tokens.is_some_and(|window| {
                window > 0
                    && self
                        .per_question_tokens
                        .is_none_or(|cap| cap > 0 && cap <= window)
            }),
        }
    }
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionContextSourceV1 {
    User,
    Metadata,
    Fallback,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionContextBasisV1 {
    Documented,
    Inferred,
    MaintainerPolicy,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionContextMetadataV1 {
    pub binding_key: String,
    pub interface_key: String,
    pub constraint_key: String,
    pub basis: DecisionContextBasisV1,
    pub evidence_refs: Vec<String>,
    pub note: String,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedDecisionContextV1 {
    pub request_window_tokens: u32,
    pub per_question_window_tokens: u32,
    pub request_source: DecisionContextSourceV1,
    pub per_question_source: DecisionContextSourceV1,
    pub catalog_revision: u64,
    pub catalog_digest: CanonicalDigest,
    pub metadata: Option<DecisionContextMetadataV1>,
    pub typesafe_long_context_header: bool,
    pub accounting_algorithm: String,
    pub reserve_policy: String,
    pub context_digest: CanonicalDigest,
}

impl ResolvedDecisionContextV1 {
    /// Digest the complete policy identity without its self-authenticating digest field.
    pub fn calculate_digest(&self) -> Result<CanonicalDigest, crate::CanonicalDigestError> {
        let mut value = serde_json::to_value(self).expect("context serialization");
        value
            .as_object_mut()
            .expect("context is an object")
            .remove("context_digest");
        CanonicalDigest::of(&value)
    }

    pub fn validate(&self) -> bool {
        self.request_window_tokens > 0
            && self.per_question_window_tokens > 0
            && self.per_question_window_tokens <= self.request_window_tokens
            && self.accounting_algorithm == DECISION_CONTEXT_ALGORITHM_V1
            && self.reserve_policy == DECISION_CONTEXT_RESERVE_V1
            && self.catalog_revision > 0
            && self.catalog_digest != CanonicalDigest::of_bytes(&[])
            && self
                .calculate_digest()
                .is_ok_and(|digest| digest == self.context_digest)
            && (!matches!(self.request_source, DecisionContextSourceV1::Metadata)
                && !matches!(self.per_question_source, DecisionContextSourceV1::Metadata)
                || self.metadata.is_some())
            && self.metadata.as_ref().is_none_or(|metadata| {
                !metadata.binding_key.is_empty()
                    && !metadata.interface_key.is_empty()
                    && !metadata.constraint_key.is_empty()
                    && !metadata.evidence_refs.is_empty()
                    && metadata.note.len() <= 2048
            })
    }

    pub fn request_usable_tokens(&self) -> u32 {
        decision_context_usable_tokens(self.request_window_tokens)
    }

    pub fn per_question_usable_tokens(&self) -> u32 {
        decision_context_usable_tokens(self.per_question_window_tokens)
    }
}

pub fn decision_context_usable_tokens(window: u32) -> u32 {
    let reserve = window.div_ceil(20).max(1024).min(window);
    window - reserve
}

/// The typed provider header never follows a provider label or a compatible proxy URL.
pub fn official_typesafe_long_context_destination(
    endpoint: &str,
    protocol: DecisionWireProtocolV2,
) -> bool {
    if protocol != DecisionWireProtocolV2::SystemOne {
        return false;
    }
    url::Url::parse(endpoint).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str() == Some("api.typesafe.ai")
            && url.port_or_known_default() == Some(443)
            && url.path() == "/v1/systemone"
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
    })
}

#[cfg(test)]
mod tests;
