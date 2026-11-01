//! Non-text evidence of local allocation, including rejection before provider dispatch.
use super::{
    DECISION_CONTEXT_ALGORITHM_V1, DecisionContextSourceV1, decision_context_usable_tokens,
};
use crate::CanonicalDigest;
use serde::{Deserialize, Serialize};

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum DecisionContextObservationSchemaV1 {
    #[serde(rename = "hiroute.decision-context-observation/v1")]
    V1,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionContextConstraintV1 {
    Request,
    Question,
    JsonShape,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionContextNoFitV1 {
    MandatoryCore,
    MandatoryShape,
    Accounting,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionContextObservationV1 {
    pub schema: DecisionContextObservationSchemaV1,
    pub context_digest: CanonicalDigest,
    pub request_source: DecisionContextSourceV1,
    pub question_source: DecisionContextSourceV1,
    pub request_window_tokens: u32,
    pub question_window_tokens: u32,
    pub request_usable_tokens: u32,
    pub question_usable_tokens: u32,
    pub estimator: String,
    pub estimated_request_tokens: Option<u64>,
    pub estimated_largest_question_tokens: Option<u64>,
    pub encoded_bytes: Option<u64>,
    pub target_turns_retained: u32,
    pub target_turns_removed: u32,
    pub target_steps_retained: u32,
    pub target_steps_removed: Option<u32>,
    pub prior_turns_retained: u32,
    pub prior_turns_removed: u32,
    pub prior_steps_retained: u32,
    pub prior_steps_removed: Option<u32>,
    pub replay_messages_retained: u32,
    pub replay_messages_omitted: Option<u32>,
    pub replay_envelopes_scanned: u32,
    pub replay_envelopes_unscanned: Option<u32>,
    pub replay_truncated: bool,
    pub replay_boundary_unavailable: bool,
    pub replay_budget_omitted: bool,
    pub target_partial: bool,
    pub limiting_constraint: DecisionContextConstraintV1,
    pub no_fit_reason: Option<DecisionContextNoFitV1>,
}

impl DecisionContextObservationV1 {
    pub fn validate(&self) -> bool {
        self.request_window_tokens > 0
            && self.question_window_tokens > 0
            && self.question_window_tokens <= self.request_window_tokens
            && self.request_usable_tokens
                == decision_context_usable_tokens(self.request_window_tokens)
            && self.question_usable_tokens
                == decision_context_usable_tokens(self.question_window_tokens)
            && self.estimator == DECISION_CONTEXT_ALGORITHM_V1
            && CanonicalDigest::parse(self.context_digest.as_str()).is_ok()
            && self.prior_turns_retained <= 8
            && self.replay_messages_retained <= 16
            && self.replay_envelopes_scanned <= 256
            && if self.no_fit_reason.is_some() {
                self.encoded_bytes.is_none()
                    && self.target_turns_retained == 0
                    && self.prior_turns_retained == 0
                    && self.replay_messages_retained == 0
            } else {
                self.estimated_request_tokens
                    .is_some_and(|used| used <= u64::from(self.request_usable_tokens))
                    && self
                        .estimated_largest_question_tokens
                        .is_some_and(|used| used <= u64::from(self.question_usable_tokens))
                    && self.encoded_bytes == self.estimated_request_tokens
            }
    }
}
