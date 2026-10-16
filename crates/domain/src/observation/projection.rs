use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{AgentPlanId, CanonicalDigest, WorkspaceId};

use super::{
    AttemptId, ContentBlobDigest, ContentDownstreamDeliveryV2, ContentId, ContentRefV2,
    ConversationContentDirectionV2, EventId, ExecutionCorrelationV1, ExecutionFactV1,
    ExecutionProducerV1, ExecutionRequestOutcomeV1, FrozenExecutionTrustV1, LogicalRequestId,
    MessageInstanceId, ReceiptId, SessionId, TranscriptRoot, TurnId,
};

pub const ROUTING_RECEIPT_SCHEMA_V1: &str = "hiroute.routing-receipt/v1";
pub const VALUE_LEDGER_SCHEMA_V1: &str = "hiroute.value-ledger-entry/v1";
pub const VALUE_CALCULATION_BASIS_V1: &str = "hiroute.value-calculation/baseline-chosen-actual/v1";
pub const OBSERVATION_RETENTION_SCHEMA_V1: &str = "hiroute.observation-retention/v1";
pub const SEVEN_DAYS_MILLIS: i64 = 7 * 24 * 60 * 60 * 1_000;

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CorrelationProvenance {
    AgentSupplied,
    ProtocolState,
    GatewayGenerated,
    Unproven,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrafficKind {
    Normal,
    ConnectivityProbe,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FactsCompleteness {
    Complete,
    Partial,
    Unknown,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentCompleteness {
    Complete,
    Partial,
    NotCaptured,
    Expired,
    Deleted,
    Unknown,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationCompletenessScope {
    GatewayVisible,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentAccess {
    Authorized,
    Unauthorized,
    NotRequested,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestOutcome {
    Accepted,
    Failed,
    Cancelled,
    PostcommitPartial,
    PostcommitTransportFailed,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UsageFactsV1 {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub reasoning_tokens: u64,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenValueFactsV1 {
    pub agent_plan_id: AgentPlanId,
    pub currency: String,
    pub billing_unit: String,
    pub price_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price_override_revision: Option<String>,
    pub baseline_api_equivalent_cost_micros: Option<u64>,
    pub chosen_api_equivalent_cost_micros: Option<u64>,
    pub actual_incremental_cost_micros: Option<u64>,
    pub routing_savings_micros: Option<i64>,
    pub entitlement_savings_micros: Option<i64>,
    pub estimated_total_savings_micros: Option<i64>,
}

impl FrozenValueFactsV1 {
    pub fn validate(&self) -> Result<(), ObservationProjectionError> {
        if AgentPlanId::parse(self.agent_plan_id.as_str()).is_err()
            || self.currency.is_empty()
            || self.billing_unit.is_empty()
            || self.price_version.is_empty()
        {
            return Err(ObservationProjectionError::InvalidValue);
        }
        validate_savings(
            self.baseline_api_equivalent_cost_micros,
            self.chosen_api_equivalent_cost_micros,
            self.routing_savings_micros,
        )?;
        validate_savings(
            self.chosen_api_equivalent_cost_micros,
            self.actual_incremental_cost_micros,
            self.entitlement_savings_micros,
        )?;
        validate_savings(
            self.baseline_api_equivalent_cost_micros,
            self.actual_incremental_cost_micros,
            self.estimated_total_savings_micros,
        )
    }
}

fn validate_savings(
    minuend: Option<u64>,
    subtrahend: Option<u64>,
    savings: Option<i64>,
) -> Result<(), ObservationProjectionError> {
    let expected = match (minuend, subtrahend) {
        (Some(minuend), Some(subtrahend)) => {
            let minuend =
                i64::try_from(minuend).map_err(|_| ObservationProjectionError::InvalidValue)?;
            let subtrahend =
                i64::try_from(subtrahend).map_err(|_| ObservationProjectionError::InvalidValue)?;
            Some(
                minuend
                    .checked_sub(subtrahend)
                    .ok_or(ObservationProjectionError::InvalidValue)?,
            )
        }
        _ => None,
    };
    if savings == expected {
        Ok(())
    } else {
        Err(ObservationProjectionError::InvalidValue)
    }
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiptExecutionEventV1 {
    pub sequence: u64,
    pub event_id: EventId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempt_id: Option<AttemptId>,
    pub occurred_at_unix_nanos: u64,
    pub fact: ExecutionFactV1,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingReceiptV1 {
    pub schema: String,
    pub receipt_id: ReceiptId,
    pub workspace_id: WorkspaceId,
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub request_id: LogicalRequestId,
    pub producer: ExecutionProducerV1,
    pub correlation: ExecutionCorrelationV1,
    pub trust: FrozenExecutionTrustV1,
    pub ordered_facts: Vec<ReceiptExecutionEventV1>,
    pub outcome: RequestOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_attempt_id: Option<AttemptId>,
    pub facts_completeness: FactsCompleteness,
    pub frozen_at_ms: i64,
}

impl RoutingReceiptV1 {
    pub fn validate(&self) -> Result<(), ObservationProjectionError> {
        if self.schema != ROUTING_RECEIPT_SCHEMA_V1
            || self.workspace_id != self.correlation.workspace_id
            || self.session_id != self.correlation.conversation_id
            || self.turn_id != self.correlation.turn_id
            || self.request_id != self.correlation.request_id
            || self.ordered_facts.is_empty()
            || self.frozen_at_ms <= 0
            || self.trust.validate().is_err()
        {
            return Err(ObservationProjectionError::InvalidReceipt);
        }
        let mut prior_sequence = 0;
        let mut event_ids = BTreeSet::new();
        let mut attempt_ordinals = BTreeSet::new();
        let mut attempt_ids = BTreeMap::new();
        let mut finished_attempt_ids = BTreeSet::new();
        let mut route_count = 0;
        let mut finish = None;
        for event in &self.ordered_facts {
            if event.sequence <= prior_sequence
                || event.occurred_at_unix_nanos == 0
                || !event_ids.insert(&event.event_id)
                || event.fact.validate().is_err()
            {
                return Err(ObservationProjectionError::InvalidReceipt);
            }
            prior_sequence = event.sequence;
            let attempt_scoped = matches!(
                event.fact,
                ExecutionFactV1::AttemptStarted { .. }
                    | ExecutionFactV1::AttemptFinished(_)
                    | ExecutionFactV1::SemanticCommit { .. }
                    | ExecutionFactV1::UsageAndCache { .. }
            );
            if attempt_scoped != event.attempt_id.is_some() {
                return Err(ObservationProjectionError::InvalidReceipt);
            }
            match &event.fact {
                ExecutionFactV1::RouteDecision(_) => route_count += 1,
                ExecutionFactV1::AttemptStarted { ordinal, .. } => {
                    if !attempt_ordinals.insert(*ordinal)
                        || attempt_ids
                            .insert(
                                event
                                    .attempt_id
                                    .as_ref()
                                    .ok_or(ObservationProjectionError::InvalidReceipt)?,
                                *ordinal,
                            )
                            .is_some()
                    {
                        return Err(ObservationProjectionError::InvalidReceipt);
                    }
                }
                ExecutionFactV1::AttemptFinished(finished) => {
                    let attempt_id = event
                        .attempt_id
                        .as_ref()
                        .ok_or(ObservationProjectionError::InvalidReceipt)?;
                    if attempt_ids
                        .get(attempt_id)
                        .is_some_and(|ordinal| *ordinal != finished.ordinal)
                        || !finished_attempt_ids.insert(attempt_id)
                    {
                        return Err(ObservationProjectionError::InvalidReceipt);
                    }
                }
                ExecutionFactV1::SemanticCommit { ordinal, .. }
                | ExecutionFactV1::UsageAndCache { ordinal, .. } => {
                    if attempt_ids
                        .get(
                            event
                                .attempt_id
                                .as_ref()
                                .ok_or(ObservationProjectionError::InvalidReceipt)?,
                        )
                        .is_some_and(|started| started != ordinal)
                    {
                        return Err(ObservationProjectionError::InvalidReceipt);
                    }
                }
                ExecutionFactV1::RequestFinished {
                    outcome,
                    attempts_started,
                    attempts_finished,
                    accepted_attempt_ordinal,
                    ..
                } => {
                    let None = finish else {
                        return Err(ObservationProjectionError::InvalidReceipt);
                    };
                    finish = Some((
                        outcome,
                        *attempts_started,
                        *attempts_finished,
                        *accepted_attempt_ordinal,
                    ));
                }
                _ => {}
            }
        }
        if self
            .final_attempt_id
            .as_ref()
            .is_some_and(|id| !attempt_ids.contains_key(id))
        {
            return Err(ObservationProjectionError::InvalidReceipt);
        }
        let Some((outcome, attempts_started, attempts_finished, accepted_ordinal)) = finish else {
            return Err(ObservationProjectionError::InvalidReceipt);
        };
        if route_count > 1
            || !request_outcome_matches(*outcome, self.outcome)
            || !matches!(
                self.ordered_facts.last().map(|event| &event.fact),
                Some(ExecutionFactV1::RequestFinished { .. })
            )
        {
            return Err(ObservationProjectionError::InvalidReceipt);
        }
        if self.facts_completeness == FactsCompleteness::Complete {
            let accepted_attempt = accepted_ordinal.and_then(|ordinal| {
                attempt_ids
                    .iter()
                    .find_map(|(attempt_id, value)| (*value == ordinal).then_some(*attempt_id))
            });
            if route_count != 1
                || usize::try_from(attempts_started).ok() != Some(attempt_ids.len())
                || usize::try_from(attempts_finished).ok() != Some(finished_attempt_ids.len())
                || accepted_ordinal.is_some() != accepted_attempt.is_some()
                || accepted_attempt.is_some_and(|id| !finished_attempt_ids.contains(id))
                || self.final_attempt_id.as_ref() != accepted_attempt
            {
                return Err(ObservationProjectionError::InvalidReceipt);
            }
        }
        Ok(())
    }

    pub fn digest(&self) -> Result<CanonicalDigest, ObservationProjectionError> {
        self.validate()?;
        CanonicalDigest::of(self).map_err(|_| ObservationProjectionError::Encoding)
    }
}

const fn request_outcome_matches(
    source: ExecutionRequestOutcomeV1,
    projected: RequestOutcome,
) -> bool {
    matches!(
        (source, projected),
        (
            ExecutionRequestOutcomeV1::Accepted,
            RequestOutcome::Accepted
        ) | (ExecutionRequestOutcomeV1::Failed, RequestOutcome::Failed)
            | (
                ExecutionRequestOutcomeV1::Cancelled,
                RequestOutcome::Cancelled
            )
            | (
                ExecutionRequestOutcomeV1::PostcommitPartial,
                RequestOutcome::PostcommitPartial
            )
            | (
                ExecutionRequestOutcomeV1::PostcommitTransportFailed,
                RequestOutcome::PostcommitTransportFailed
            )
    )
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ValueLedgerEntryV1 {
    pub schema: String,
    pub workspace_id: WorkspaceId,
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub request_id: LogicalRequestId,
    pub receipt_id: ReceiptId,
    pub usage: UsageFactsV1,
    pub frozen: FrozenValueFactsV1,
    pub facts_completeness: FactsCompleteness,
    pub frozen_at_ms: i64,
}

impl ValueLedgerEntryV1 {
    pub fn validate(&self) -> Result<(), ObservationProjectionError> {
        if self.schema != VALUE_LEDGER_SCHEMA_V1 {
            return Err(ObservationProjectionError::InvalidValue);
        }
        self.frozen.validate()
    }
}

/// Zero-content aggregate retained after request-level observation details expire.
///
/// The bucket intentionally has no Session, Turn, Request, Receipt, model, or content reference,
/// so it cannot be used to reconstruct an expired timeline.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DailyValueAggregateV1 {
    pub agent_plan_id: AgentPlanId,
    pub day_number: i64,
    pub currency: String,
    pub billing_unit: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub reasoning_tokens: u64,
    pub baseline_api_equivalent_cost_micros: Option<u64>,
    pub chosen_api_equivalent_cost_micros: Option<u64>,
    pub actual_incremental_cost_micros: Option<u64>,
    pub routing_savings_micros: Option<i64>,
    pub entitlement_savings_micros: Option<i64>,
    pub estimated_total_savings_micros: Option<i64>,
    pub price_version_refs: BTreeSet<String>,
    pub price_override_revision_refs: BTreeSet<String>,
    pub facts_completeness: FactsCompleteness,
    pub detail_available: bool,
}

#[cfg(test)]
mod value_tests {
    use super::*;

    fn value(
        baseline: Option<u64>,
        chosen: Option<u64>,
        actual: Option<u64>,
        routing: Option<i64>,
        entitlement: Option<i64>,
        total: Option<i64>,
    ) -> FrozenValueFactsV1 {
        FrozenValueFactsV1 {
            agent_plan_id: AgentPlanId::parse("plan/value-test").unwrap(),
            currency: "USD".to_owned(),
            billing_unit: "micros".to_owned(),
            price_version: "price-v1".to_owned(),
            price_override_revision: None,
            baseline_api_equivalent_cost_micros: baseline,
            chosen_api_equivalent_cost_micros: chosen,
            actual_incremental_cost_micros: actual,
            routing_savings_micros: routing,
            entitlement_savings_micros: entitlement,
            estimated_total_savings_micros: total,
        }
    }

    #[test]
    fn value_semantics_accept_negative_savings_and_propagate_unknown() {
        assert!(
            value(
                Some(100),
                Some(150),
                Some(175),
                Some(-50),
                Some(-25),
                Some(-75)
            )
            .validate()
            .is_ok()
        );
        assert!(
            value(None, Some(150), Some(175), None, Some(-25), None)
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn value_semantics_reject_fabricated_or_inconsistent_savings() {
        assert!(
            value(None, Some(150), Some(175), Some(0), Some(-25), None)
                .validate()
                .is_err()
        );
        assert!(
            value(
                Some(100),
                Some(150),
                Some(175),
                Some(-50),
                Some(0),
                Some(-75)
            )
            .validate()
            .is_err()
        );
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentDirection {
    Request,
    Response,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentKind {
    Message,
    ToolDefinition,
    ToolCall,
    ToolResult,
    ToolError,
}

impl ContentKind {
    pub const fn is_tool(self) -> bool {
        !matches!(self, Self::Message)
    }
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StoredMessageV1 {
    pub request_id: LogicalRequestId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempt_id: Option<AttemptId>,
    pub fork_id: String,
    pub message_instance_id: MessageInstanceId,
    pub content_id: ContentId,
    pub blob_digest: ContentBlobDigest,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transcript_root: Option<TranscriptRoot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_transcript_root: Option<TranscriptRoot>,
    pub direction: ConversationContentDirectionV2,
    pub message_ordinal: u32,
    pub part_ordinal: u32,
    pub kind: String,
    pub role: String,
    pub media_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transport_frame_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_ref: Option<ContentRefV2>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub downstream_delivery: Option<ContentDownstreamDeliveryV2>,
    pub occurred_at_unix_nanos: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bytes: Vec<u8>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RetentionPolicyV1 {
    pub schema: String,
    pub detail_retention_ms: i64,
    pub content_retention_ms: i64,
}

impl Default for RetentionPolicyV1 {
    fn default() -> Self {
        Self {
            schema: OBSERVATION_RETENTION_SCHEMA_V1.to_owned(),
            detail_retention_ms: SEVEN_DAYS_MILLIS,
            content_retention_ms: SEVEN_DAYS_MILLIS,
        }
    }
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TombstoneReason {
    RetentionExpired,
    UserDeleted,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ObservationProjectionError {
    #[error("routing receipt is invalid")]
    InvalidReceipt,
    #[error("value facts are invalid")]
    InvalidValue,
    #[error("observation projection could not be canonically encoded")]
    Encoding,
}
