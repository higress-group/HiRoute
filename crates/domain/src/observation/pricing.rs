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
pub const EXECUTION_PRODUCT_CONTRACT: &str = r#"{"schema":"hiroute.observation.product-execution-envelope/v2","extends_digest":"sha256:5aad4c450a2a296ec557b7a934bfbf70ad69a8e8fe9bc7539db46830a2940069","optional_field":"pricing","pricing_schema":"hiroute.observation.execution-pricing/v1","pricing_fields":["usage_semantics","schema_version","request_generation?","captured_at_ms","attempt_execution_at_ms","quote?","reference_quote?","unknown_reason?"],"scope":"attempt_started_only","quote_type":"FrozenPriceQuoteV1","quote_time_unit":"unix_seconds","capture_time_unit":"unix_milliseconds","route_identity":{"replaces_trust_fields":["agent_plan_revision_id","agent_plan_semantic_digest"],"replaces_route_decision_fields":["plan_revision"],"field":"route","tag":"kind","plan":{"revision":"positive_u64","semantic_digest":"sha256"},"fixed":{"binding_digest":"sha256"},"plan_identity":"required_for_plan_null_for_fixed","fixed_plan_display_name":"forbidden","route_decision_matches_trust":true},"complexity_decision":{"fields":["extension_reason?","decision_protocol","strategy_id","schema_version","payload_digest","branch_id","policy?","competence_trigger?","execution_group","simple_probability?","simple_threshold_millis?","selection_reason","complexity_score?","threshold?","decision_source","reason_codes","matched_user_phrase_ids","fallback_used","classification_duration_micros?","fallback_reason?","routing_observation?"],"sources":["inherited","external_classifier","user_phrase","builtin_rules","unresolved"],"fallback_reasons":["timeout","unavailable","rejected_input","invalid_output"],"protocols":["system_one_legacy","system_one_c","custom_extension_v1","local_rules"]},"branch_execution":{"scope":"agent_turn_finished","fields":["policy","group","candidate_index"],"policy_fields":["name","floor_millis?","criteria_digest?","extension?"],"extension_policy":{"fields":["id","revision"],"floor":"absent","criteria_digest":"null"}},"competence_protection":{"scope":"route_decision.complexity.competence_trigger","fields":["segment_id","score","floor_millis","from_group"]},"named_tool_choice":{"kinds":["function","custom"],"function_encoding":"implicit","custom_encoding":"explicit_tool_kind","omitted_kind":"function"},"agent_turn_opening":{"scope":"agent_turn_finished","optional_field":"opening_cause","values":["user_message","history_shortened","history_rewrite","no_comparable_history","other"],"absence":"legacy_unrecorded"},"history_membership":{"scope":["agent_turn_finished","branch_assessment_recorded"],"optional_field":"history_membership","version":1,"fields":["branch_id","revision","request_ids","clipped","relation","source_unverified","association_unverified","capture_partial"],"max_requests":128,"relations":["root","exact_prefix","ambiguous_prefix","contextual"],"absence":"legacy_range_inference","revision":"immutable_snapshot"},"native_auxiliary":{"scope":"route_decision","optional_field":"native_auxiliary","default":false,"source":"recognized_native_maintenance_purpose","timeline":"exclude_without_erasing_receipt_or_accounting"},"agent_turn_admission":{"kind":"agent_turn_admitted","version":1,"fields":["version","agent_turn_id","branch_id","revision","ordinal","opening_cause"],"identity_scope":["envelope.producer_epoch","envelope.request_id"],"revision":"positive_u64","ordinal":"positive_u64","opening_cause":"observed_boundary_not_human_authorship","meaning":"admission_identity_only","does_not_prove":["segment","history_association","capture_completeness","stage_closure","logical_task_completion","human_source"],"absence":"legacy_unrecorded_never_inferred_from_route_decision_or_affinity"},"request_input_selection":{"kind":"request_input_selection","version":1,"fields":["evidence","message_ordinal?","source_unverified"],"evidence":["native_matched","wire_observed","unavailable"],"ordinal":"original canonical capture ordinal; first selected message, not full batch or human proof","retained_catalog":"explicit opt-in only"},"stage_assessment":{"kind":"stage_assessment_recorded","scope":"composite actual execution under one frozen Plan and rubric","fields":["scope","history_membership","segment_id","plan_id","plan_revision","trigger_request_id","target_from_turn_id","target_through_turn_id","target_from_ordinal","target_through_ordinal","assessed_at_ms","score","partial","reason?"],"executor_identity":"none; never a model/profile score","scope_fields":["version","executed_branch_id","policy"],"legacy":"no old digest authenticates either added fact"}}"#;

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
