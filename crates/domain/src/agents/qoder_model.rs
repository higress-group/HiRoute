//! Native budget and additional-model declarations shared by Qoder settings and Workers.
use serde::{Deserialize, Serialize};

/// Explicit local model-grant bearer channel for Qoder's fixed native auth headers.
/// The ordinary Responses endpoint retains its independent X-HiRoute-Token channel.
pub const QODER_MODEL_BASE_PATH: &str = "/_hiroute/qoder/v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("Qoder budget is invalid: {stage}")]
pub struct QoderBudgetError {
    pub stage: &'static str,
}

fn budget_error(stage: &'static str) -> QoderBudgetError {
    QoderBudgetError { stage }
}

// Qoder 1.1.65's catalogue-backed gdA/Uce and Iwe reserve at most 20K output
// tokens, then max(13K, 5% of the remaining window) for auto-compaction.
// maxOutputTokens changes that catalogue; the ACP CLI output flag does not.
const OUTPUT_RESERVATION_LIMIT: u64 = 20_000;
const COMPACTION_MARGIN: u64 = 13_000;
const MAX_NATIVE_INTEGER: u64 = (1 << 53) - 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QoderTokenBudget {
    pub context_window_tokens: u64,
    pub max_output_tokens: u64,
}

impl QoderTokenBudget {
    pub fn new(
        context_window_tokens: u64,
        max_output_tokens: u64,
    ) -> Result<Self, QoderBudgetError> {
        validate_qoder_output_budget(max_output_tokens)?;
        if context_window_tokens > MAX_NATIVE_INTEGER {
            return Err(budget_error("native token budget"));
        }
        let available = context_window_tokens
            .checked_sub(max_output_tokens.min(OUTPUT_RESERVATION_LIMIT))
            .ok_or_else(|| budget_error("native token budget"))?;
        if available <= COMPACTION_MARGIN.max(available / 20) {
            return Err(budget_error("native token budget"));
        }
        // A positive threshold is not a promise that arbitrary skills or prompts fit.
        Ok(Self {
            context_window_tokens,
            max_output_tokens,
        })
    }
}

pub fn validate_qoder_output_budget(tokens: u64) -> Result<(), QoderBudgetError> {
    if tokens == 0 || tokens > MAX_NATIVE_INTEGER {
        return Err(budget_error("native output budget"));
    }
    Ok(())
}

/// Non-secret native model declaration sealed in a settings Operation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct QoderAdditionalModelV1 {
    pub alias: String,
    pub context_window_tokens: u64,
    pub max_output_tokens: u64,
}

impl QoderAdditionalModelV1 {
    pub fn validate(&self) -> Result<(), crate::AgentConnectionError> {
        crate::ModelAlias::parse(&self.alias)
            .map_err(|_| crate::AgentConnectionError::InvalidGrant)?;
        QoderTokenBudget::new(self.context_window_tokens, self.max_output_tokens)
            .map_err(|_| crate::AgentConnectionError::InvalidGrant)?;
        Ok(())
    }
}
