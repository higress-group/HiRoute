//! Portable V2 observation scope and query contracts. Reader contexts cannot be
//! deserialized from transport; only a trusted admission service constructs them.
use crate::{
    AgentPlanDisplayName, AgentPlanId, LogicalRequestId, ObservationQueryError, WorkspaceId,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug)]
pub struct ObservationReaderContext {
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    workspace: WorkspaceId,
    subject: String,
    allowed_runs: Option<BTreeSet<String>>,
    read_content: bool,
    search_content: bool,
    authorization_generation: u64,
    expires_at_ms: i64,
}

impl ObservationReaderContext {
    /// Cancels this observation only; never cancels an Agent, Worker or model.
    pub fn cancel(&self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(std::sync::atomic::Ordering::Acquire)
    }

    pub fn workspace(&self) -> &WorkspaceId {
        &self.workspace
    }
    pub fn allowed_runs(&self) -> Option<&BTreeSet<String>> {
        self.allowed_runs.as_ref()
    }
    /// Only a trusted local-user authorization issuer may construct this context.
    pub fn local_user(
        workspace: WorkspaceId,
        subject: String,
        authorization_generation: u64,
        expires_at_ms: i64,
        read_content: bool,
        search_content: bool,
    ) -> Result<Self, ObservationQueryError> {
        if !identifier(&subject)
            || expires_at_ms < 0
            || WorkspaceId::parse(workspace.as_str()).is_err()
        {
            return Err(ObservationQueryError::InvalidQuery);
        }
        Ok(Self {
            cancelled: Default::default(),
            workspace,
            subject,
            allowed_runs: None,
            read_content,
            search_content,
            authorization_generation,
            expires_at_ms,
        })
    }

    /// Caller must supply the allowed runs from current verified run admission.
    pub fn run_scoped(
        workspace: WorkspaceId,
        subject: String,
        authorization_generation: u64,
        expires_at_ms: i64,
        allowed_runs: BTreeSet<String>,
        read_content: bool,
        search_content: bool,
    ) -> Result<Self, ObservationQueryError> {
        if allowed_runs.is_empty()
            || allowed_runs.len() > 200
            || allowed_runs.iter().any(|id| !identifier(id))
        {
            return Err(ObservationQueryError::InvalidQuery);
        }
        let mut reader = Self::local_user(
            workspace,
            subject,
            authorization_generation,
            expires_at_ms,
            read_content,
            search_content,
        )?;
        reader.allowed_runs = Some(allowed_runs);
        Ok(reader)
    }

    pub fn check(
        &self,
        now_ms: i64,
        content: bool,
        search: bool,
    ) -> Result<(), ObservationQueryError> {
        if self.is_cancelled() {
            return Err(ObservationQueryError::Unavailable);
        }
        if now_ms < 0
            || now_ms >= self.expires_at_ms
            || content && !self.read_content
            || search && !self.search_content
        {
            return Err(ObservationQueryError::Unauthorized);
        }
        Ok(())
    }

    pub fn binding(&self) -> Result<String, ObservationQueryError> {
        crate::CanonicalDigest::of(&(
            &self.workspace,
            &self.subject,
            &self.allowed_runs,
            self.read_content,
            self.search_content,
            self.authorization_generation,
        ))
        .map(|digest| digest.to_string())
        .map_err(|_| ObservationQueryError::InvalidQuery)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunObservationLink {
    pub workspace_id: WorkspaceId,
    pub request_id: LogicalRequestId,
    pub task_id: String,
    pub run_id: String,
    pub producer_epoch: String,
    pub source_event_id: String,
    pub plan_id: String,
    pub plan_revision: String,
    pub publication_ref: String,
    pub harness_id: String,
    pub protocol_kind: String,
    pub native_session_id: Option<String>,
    pub native_turn_id: Option<String>,
    /// Opaque caller-supplied provenance only; it is not a verified parent session or authority.
    pub parent_context_ref: Option<String>,
    pub continued_from_run_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationRequestQuery {
    pub from_ms: i64,
    pub to_ms: i64,
    pub session_id: Option<String>,
    /// Optional exact request locator. This is independent of page position so a
    /// persisted quality sample can open evidence that is no longer on the first
    /// timeline page.
    #[serde(default)]
    pub request_id: Option<String>,
    pub limit: u16,
    pub cursor: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub plan_id: Option<String>,
    #[serde(default)]
    pub native_model: Option<String>,
    #[serde(default)]
    pub outcome: Option<String>,
    #[serde(default)]
    pub only_model_switch: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationRoutingContextStateV1 {
    Recorded,
    RecordedFixed,
    Unavailable,
    Conflicted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationRoutingContextNameStateV1 {
    Recorded,
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationRoutingContextV1 {
    pub state: ObservationRoutingContextStateV1,
    pub plan_id: Option<String>,
    pub plan_revision: Option<String>,
    pub display_name: Option<String>,
    pub name_state: ObservationRoutingContextNameStateV1,
}

impl ObservationRoutingContextV1 {
    pub fn from_execution_trust(trust: &super::FrozenExecutionTrustV1) -> Self {
        match (&trust.route, &trust.agent_plan_id) {
            (crate::ModelRequestRouteV2::Plan { revision, .. }, Some(plan_id)) => Self::recorded(
                plan_id.as_str().to_owned(),
                revision.to_string(),
                trust.plan_display_name.clone(),
            ),
            (crate::ModelRequestRouteV2::Fixed { .. }, None)
                if trust.plan_display_name.is_none() =>
            {
                Self {
                    state: ObservationRoutingContextStateV1::RecordedFixed,
                    ..Self::unavailable()
                }
            }
            _ => Self::unavailable(),
        }
    }

    pub fn unavailable() -> Self {
        Self {
            state: ObservationRoutingContextStateV1::Unavailable,
            plan_id: None,
            plan_revision: None,
            display_name: None,
            name_state: ObservationRoutingContextNameStateV1::Unavailable,
        }
    }

    pub fn conflicted() -> Self {
        Self {
            state: ObservationRoutingContextStateV1::Conflicted,
            ..Self::unavailable()
        }
    }

    pub fn recorded(plan_id: String, plan_revision: String, display_name: Option<String>) -> Self {
        let name_state = if display_name.is_some() {
            ObservationRoutingContextNameStateV1::Recorded
        } else {
            ObservationRoutingContextNameStateV1::Unavailable
        };
        Self {
            state: ObservationRoutingContextStateV1::Recorded,
            plan_id: Some(plan_id),
            plan_revision: Some(plan_revision),
            display_name,
            name_state,
        }
    }

    pub fn valid(&self) -> bool {
        match self.state {
            ObservationRoutingContextStateV1::Recorded => {
                self.plan_id
                    .as_deref()
                    .is_some_and(|value| AgentPlanId::parse(value).is_ok())
                    && self.plan_revision.as_deref().is_some_and(|value| {
                        value
                            .parse::<u64>()
                            .is_ok_and(|revision| revision > 0 && revision.to_string() == value)
                    })
                    && match (&self.display_name, &self.name_state) {
                        (Some(name), ObservationRoutingContextNameStateV1::Recorded) => {
                            AgentPlanDisplayName::parse(name.clone()).is_ok()
                        }
                        (None, ObservationRoutingContextNameStateV1::Unavailable) => true,
                        _ => false,
                    }
            }
            ObservationRoutingContextStateV1::RecordedFixed
            | ObservationRoutingContextStateV1::Unavailable
            | ObservationRoutingContextStateV1::Conflicted => {
                self.plan_id.is_none()
                    && self.plan_revision.is_none()
                    && self.display_name.is_none()
                    && self.name_state == ObservationRoutingContextNameStateV1::Unavailable
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObservedRequestSummary {
    pub session_id: String,
    pub request_id: String,
    pub started_at_ms: i64,
    pub outcome: Option<String>,
    pub receipt_id: Option<String>,
    pub run_id: Option<String>,
    /// Opaque caller-provided navigation provenance, never interpreted as authority.
    pub parent_context_ref: Option<String>,
    /// Internal generated turn ids are deliberately absent.
    pub native_turn_id: Option<String>,
    pub relation_conflicted: bool,
    pub routing_context: ObservationRoutingContextV1,
    pub attempted_model_count: u32,
    pub within_request_fallback: Option<bool>,
    pub final_native_model: Option<String>,
    /// None means adjacency or accepted-model evidence is incomplete.
    pub between_turn_model_change: Option<bool>,
    pub previous_native_turn_id: Option<String>,
    pub previous_turn_model: Option<String>,
    pub turn_final_native_model: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObservationRequestPage {
    pub requests: Vec<ObservedRequestSummary>,
    pub next_cursor: Option<String>,
}

fn identifier(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scope_and_permissions_bind_cursors_and_expiry_is_exclusive() {
        let reader = ObservationReaderContext::run_scoped(
            WorkspaceId::default(),
            "worker".into(),
            1,
            100,
            ["run-a".into()].into_iter().collect(),
            false,
            false,
        )
        .unwrap();
        assert!(reader.check(99, false, false).is_ok());
        assert_eq!(
            reader.check(100, false, false),
            Err(ObservationQueryError::Unauthorized)
        );
        assert_eq!(
            reader.check(1, true, false),
            Err(ObservationQueryError::Unauthorized)
        );
        assert_eq!(
            reader.check(1, false, true),
            Err(ObservationQueryError::Unauthorized)
        );
        let other = ObservationReaderContext::run_scoped(
            WorkspaceId::default(),
            "worker".into(),
            1,
            100,
            ["run-b".into()].into_iter().collect(),
            false,
            false,
        )
        .unwrap();
        assert_ne!(reader.binding().unwrap(), other.binding().unwrap());
        let revoked = ObservationReaderContext::run_scoped(
            WorkspaceId::default(),
            "worker".into(),
            2,
            100,
            ["run-a".into()].into_iter().collect(),
            false,
            false,
        )
        .unwrap();
        assert_ne!(reader.binding().unwrap(), revoked.binding().unwrap());
    }

    #[test]
    fn legacy_session_summary_defaults_correlation_to_unknown() {
        let summary: ObservationSessionSummaryV2 = serde_json::from_value(serde_json::json!({
            "session_id":"session",
            "agent_id":"",
            "first_request_at_ms":1,
            "last_request_at_ms":2,
            "request_count":1,
            "fallback_request_count":0,
            "unknown_model_request_count":0,
            "content_completeness":"unknown"
        }))
        .unwrap();
        assert_eq!(
            summary.correlation_kind,
            ObservationSessionCorrelationKindV1::Unknown
        );
    }

    #[test]
    fn routing_context_wire_shape_rejects_guessed_or_inconsistent_history() {
        let exact = ObservationRoutingContextV1::recorded(
            "plan/coding".into(),
            "17".into(),
            Some("Coding route".into()),
        );
        assert!(exact.valid());
        let value = serde_json::to_value(&exact).unwrap();
        assert_eq!(value["state"], "recorded");
        assert_eq!(value["name_state"], "recorded");

        let mut invalid_revision = exact.clone();
        invalid_revision.plan_revision = Some("017".into());
        assert!(!invalid_revision.valid());
        let mut inconsistent = exact;
        inconsistent.display_name = None;
        assert!(!inconsistent.valid());
        assert!(ObservationRoutingContextV1::conflicted().valid());
    }
}

/// A small current valuation projection. Quote and attempt detail are separate
/// from this summary so a request with many attempts cannot expand one response.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationValuationSummaryV2 {
    pub schema: String,
    pub request_id: LogicalRequestId,
    pub input_revision: u64,
    pub settled_revision: Option<u64>,
    pub pending: bool,
    pub terminal_observed: bool,
    pub facts_partial: bool,
    pub unknown_attempt_count: Option<u64>,
    pub amounts: Vec<ObservationValuationAmountV2>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationValuationAmountV2 {
    pub currency: String,
    pub valuation_kind: crate::PriceValuationKindV1,
    pub known_sum_micros: Option<u64>,
    pub coverage: ObservationMetricCoverageV2,
    pub missing_attempt_count: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationMetricCoverageV2 {
    Complete,
    Partial,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationSearchQueryV2 {
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub plan_id: Option<String>,
    #[serde(default)]
    pub native_model: Option<String>,
    #[serde(default)]
    pub outcome: Option<String>,
    #[serde(default)]
    pub only_model_switch: bool,
    pub from_ms: i64,
    pub to_ms: i64,
    pub session_id: Option<String>,
    pub keyword: String,
    pub limit: u16,
    pub cursor: Option<String>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationTextAnchorV2 {
    pub session_id: String,
    pub request_id: String,
    pub native_turn_id: Option<String>,
    pub message_occurrence_id: String,
    pub content_id: String,
    pub original_text_offset: u64,
    pub match_kind: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationSearchPageV2 {
    pub hits: Vec<ObservationTextAnchorV2>,
    pub next_cursor: Option<String>,
    pub index_partial: bool,
    pub budget_exhausted: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationContentQueryV2 {
    #[serde(default)]
    pub anchor_offset: Option<u64>,
    pub request_id: LogicalRequestId,
    pub content_id: String,
    pub cursor: Option<String>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationContentPageV2 {
    pub request_id: LogicalRequestId,
    pub content_id: String,
    pub message_occurrence_id: String,
    pub media_type: String,
    pub original_digest: String,
    pub original_byte_count: u64,
    pub direction: String,
    pub downstream_delivery: Option<String>,
    pub state: String,
    pub chunks: Vec<ObservationTextChunkV2>,
    pub next_cursor: Option<String>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationTextChunkV2 {
    pub original_byte_offset: u64,
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationFactsQueryV2 {
    pub request_id: LogicalRequestId,
    pub limit: u16,
    pub cursor: Option<String>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationSafeFactV2 {
    pub event_id: String,
    pub sequence: u64,
    pub original_digest: String,
    /// Safe routing identity projected from the request-frozen execution trust. This remains
    /// after sensitive fact and receipt bodies are deleted; legacy rows decode without it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing_context: Option<ObservationRoutingContextV1>,
    pub occurred_at_ms: Option<i64>,
    pub event_kind: String,
    pub attempt_ordinal: Option<u32>,
    pub native_model: Option<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub outcome: Option<String>,
    pub sensitive_fields_deleted: bool,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationFactsPageV2 {
    pub facts: Vec<ObservationSafeFactV2>,
    pub next_cursor: Option<String>,
    pub projection_partial: bool,
}

/// Session-level summaries contain only observations visible to this reader.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObservationSessionPageV2 {
    pub sessions: Vec<ObservationSessionSummaryV2>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationSessionCorrelationKindV1 {
    AgentSupplied,
    VerifiedWorker,
    Inferred,
    RequestScoped,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObservationSessionSummaryV2 {
    pub session_id: String,
    pub agent_id: String,
    pub first_request_at_ms: i64,
    pub last_request_at_ms: i64,
    pub request_count: u64,
    pub fallback_request_count: u64,
    pub unknown_model_request_count: u64,
    pub content_completeness: super::ContentCompleteness,
    #[serde(default)]
    pub correlation_kind: ObservationSessionCorrelationKindV1,
}
