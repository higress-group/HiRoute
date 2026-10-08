//! Optional frozen pricing evidence on an actual AttemptStarted event. No price
//! lookup or mutable source state is retained by the asynchronous writer.
use crate::{FrozenPriceQuoteV1, PriceGenerationRefV1, PriceUnknownReasonV1};
use serde::{Deserialize, Serialize};

pub const EXECUTION_PRICING_SCHEMA_V1: &str = "hiroute.observation.execution-pricing/v1";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionPricingEvidenceV1 {
    #[serde(default)]
    pub usage_semantics: UsageSemanticsV1,
    pub schema_version: String,
    pub request_generation: Option<PriceGenerationRefV1>,
    pub captured_at_ms: i64,
    pub attempt_execution_at_ms: i64,
    pub quote: Option<FrozenPriceQuoteV1>,
    pub reference_quote: Option<FrozenPriceQuoteV1>,
    pub unknown_reason: Option<PriceUnknownReasonV1>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageFrameKindV1 {
    Cumulative,
    Delta,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputUsageMeaningV1 {
    IncludesExclusiveCache,
    UncachedOnly,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputUsageMeaningV1 {
    IncludesReasoning,
    ReasoningSeparatelyAtOutputRate,
    #[default]
    Unknown,
}

/// Proven protocol/decoder facts; absent metadata never means the common case.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageSemanticsV1 {
    pub frame_kind: UsageFrameKindV1,
    pub input: InputUsageMeaningV1,
    pub output: OutputUsageMeaningV1,
    pub cache_buckets_exclusive: bool,
}

impl ExecutionPricingEvidenceV1 {
    pub fn validate(&self) -> Result<(), super::ExecutionFactError> {
        use super::ExecutionFactError;
        if self.schema_version != EXECUTION_PRICING_SCHEMA_V1
            || self.captured_at_ms < 0
            || self.attempt_execution_at_ms < self.captured_at_ms
            || self.quote.is_none() && self.unknown_reason.is_none()
        {
            return Err(ExecutionFactError::InvalidFact);
        }
        for (quote, time) in [
            (&self.quote, self.attempt_execution_at_ms),
            (&self.reference_quote, self.captured_at_ms),
        ] {
            if let Some(quote) = quote {
                quote
                    .verify_digest()
                    .map_err(|_| ExecutionFactError::InvalidFact)?;
                quote
                    .exact_target
                    .validate()
                    .map_err(|_| ExecutionFactError::InvalidFact)?;
                if quote.generation_ref != self.request_generation
                    || quote.attempt_execution_at != time / 1000
                {
                    return Err(ExecutionFactError::InvalidFact);
                }
            }
        }
        Ok(())
    }
}

/// One Product contract for priced and unpriced execution. The optional field records whether
/// pricing evidence existed; absence never selects another envelope version.
pub const EXECUTION_PRODUCT_CONTRACT: &str = r#"{"schema":"hiroute.observation.product-execution-envelope/v2","extends_digest":"sha256:5aad4c450a2a296ec557b7a934bfbf70ad69a8e8fe9bc7539db46830a2940069","optional_field":"pricing","pricing_schema":"hiroute.observation.execution-pricing/v1","pricing_fields":["usage_semantics","schema_version","request_generation?","captured_at_ms","attempt_execution_at_ms","quote?","reference_quote?","unknown_reason?"],"scope":"attempt_started_only","quote_type":"FrozenPriceQuoteV1","quote_time_unit":"unix_seconds","capture_time_unit":"unix_milliseconds","route_identity":{"replaces_trust_fields":["agent_plan_revision_id","agent_plan_semantic_digest"],"replaces_route_decision_fields":["plan_revision"],"field":"route","tag":"kind","plan":{"revision":"positive_u64","semantic_digest":"sha256"},"fixed":{"binding_digest":"sha256"},"plan_identity":"required_for_plan_null_for_fixed","fixed_plan_display_name":"forbidden","route_decision_matches_trust":true},"complexity_decision":{"fields":["strategy_id","schema_version","payload_digest","branch_id","policy?","competence_trigger?","execution_group","simple_probability?","simple_threshold_millis?","selection_reason","complexity_score?","threshold?","decision_source","reason_codes","matched_user_phrase_ids","fallback_used","classification_duration_micros?","fallback_reason?"],"sources":["inherited","external_classifier","user_phrase","builtin_rules","unresolved"],"fallback_reasons":["timeout","unavailable","rejected_input","invalid_output"]},"branch_execution":{"scope":"agent_turn_finished","fields":["policy","group","candidate_index"],"policy_fields":["name","floor_millis","criteria_digest?"]},"competence_protection":{"scope":"route_decision.complexity.competence_trigger","fields":["segment_id","score","floor_millis","from_group"]}}"#;

#[cfg(test)]
mod tests {
    #[test]
    fn product_pricing_contract_digest_is_reproducible() {
        assert_eq!(
            crate::CanonicalDigest::of_bytes(super::EXECUTION_PRODUCT_CONTRACT.as_bytes()).as_str(),
            crate::EXECUTION_FACT_PORT_DIGEST_V2
        );
    }
}
