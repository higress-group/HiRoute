use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{AgentPlanId, CanonicalDigest, WorkspaceId};

use super::{
    ContentAccess, ContentCompleteness, CorrelationProvenance, DailyValueAggregateV1,
    FactsCompleteness, ObservationCompletenessScope, ReceiptId, RetentionPolicyV1,
    RoutingReceiptV1, SessionId, StoredMessageV1, TombstoneReason, TurnId, ValueLedgerEntryV1,
};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationCapabilityV1 {
    ReadFacts,
    ReadContent,
    SearchContent,
    ManageRetention,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationPrincipalV1 {
    pub workspace_id: WorkspaceId,
    pub capabilities: BTreeSet<ObservationCapabilityV1>,
}

impl ObservationPrincipalV1 {
    pub fn local_user(workspace_id: WorkspaceId) -> Self {
        Self {
            workspace_id,
            capabilities: [
                ObservationCapabilityV1::ReadFacts,
                ObservationCapabilityV1::ReadContent,
                ObservationCapabilityV1::SearchContent,
                ObservationCapabilityV1::ManageRetention,
            ]
            .into_iter()
            .collect(),
        }
    }

    pub fn facts_only(workspace_id: WorkspaceId) -> Self {
        Self {
            workspace_id,
            capabilities: [ObservationCapabilityV1::ReadFacts].into_iter().collect(),
        }
    }

    pub fn allows(&self, capability: ObservationCapabilityV1) -> bool {
        self.capabilities.contains(&capability)
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentMode {
    #[default]
    None,
    Messages,
    MessagesAndTools,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelSwitchFilter {
    #[default]
    Any,
    Only,
    Exclude,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionListQueryV1 {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default)]
    pub model_switch: ModelSwitchFilter,
    #[serde(default)]
    pub include_unlinked: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionSummaryV1 {
    pub session_id: SessionId,
    pub agent_id: String,
    pub correlation_provenance: CorrelationProvenance,
    pub started_at_ms: i64,
    pub updated_at_ms: i64,
    pub retention_deadline_ms: i64,
    pub turn_count: u64,
    pub request_count: u64,
    pub model_switch: Option<bool>,
    pub facts_completeness: FactsCompleteness,
    pub content_completeness: ContentCompleteness,
    pub completeness_scope: ObservationCompletenessScope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tombstone_reason: Option<TombstoneReason>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionListV1 {
    pub sessions: Vec<SessionSummaryV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TurnDetailV1 {
    pub turn_id: TurnId,
    pub started_at_ms: i64,
    pub request_ids: Vec<super::LogicalRequestId>,
    pub receipt_ids: Vec<ReceiptId>,
    pub messages: Vec<StoredMessageV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionDetailV1 {
    pub summary: SessionSummaryV1,
    pub turns: Vec<TurnDetailV1>,
    pub content_access: ContentAccess,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueGroupByV1 {
    None,
    Day,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ValueQueryV1 {
    pub agent_plan_id: AgentPlanId,
    pub from_ms: i64,
    pub to_ms: i64,
    pub group_by: ValueGroupByV1,
    pub currency: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ValueViewV1 {
    pub value_calculation_basis: String,
    pub agent_plan_id: AgentPlanId,
    pub currency: String,
    pub group_by: ValueGroupByV1,
    pub groups: Vec<ValueGroupedSummaryV1>,
    pub entries: Vec<ValueLedgerEntryV1>,
    pub daily_aggregates: Vec<DailyValueAggregateV1>,
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ValueGroupedSummaryV1 {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub day_number: Option<i64>,
    pub billing_unit_refs: BTreeSet<String>,
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationGapV1 {
    pub channel: super::ObservationChannel,
    pub stream: super::ObservationStreamV1,
    pub first_sequence: u64,
    pub last_sequence: u64,
    pub known_loss: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationStatusV1 {
    pub retention: RetentionPolicyV1,
    pub store_revision: u64,
    pub facts_completeness: FactsCompleteness,
    pub content_completeness: ContentCompleteness,
    pub completeness_scope: ObservationCompletenessScope,
    pub gaps: Vec<ObservationGapV1>,
    pub gaps_truncated: bool,
    pub activity_bytes: u64,
    pub content_bytes: u64,
    pub production_exporter: String,
}

pub trait ObservationQueryPort: Send + Sync {
    fn observed_maintenance_status(
        &self,
        _reader: &super::ObservationReaderContext,
        _now_ms: i64,
    ) -> Result<super::ObservationMaintenanceStatusV2, ObservationQueryError> {
        Err(ObservationQueryError::Unavailable)
    }

    fn preview_session_deletion_v2(
        &self,
        _principal: &ObservationPrincipalV1,
        _spec: &SessionDeletionSpecV1,
        _through_ms: i64,
    ) -> Result<super::SessionDeletionPreviewV2, ObservationQueryError> {
        Err(ObservationQueryError::Unavailable)
    }
    fn apply_session_deletion_v2(
        &self,
        _principal: &ObservationPrincipalV1,
        _preview: &super::SessionDeletionPreviewV2,
        _accepted: &CanonicalDigest,
        _now_ms: i64,
    ) -> Result<super::SessionDeletionOutcomeV2, ObservationQueryError> {
        Err(ObservationQueryError::Unavailable)
    }

    fn observed_value_totals(
        &self,
        _reader: &super::ObservationReaderContext,
        _query: &super::ObservationValueQueryV2,
        _now_ms: i64,
    ) -> Result<super::ObservationValueSummaryV2, ObservationQueryError> {
        Err(ObservationQueryError::Unavailable)
    }

    fn observed_ancestry(
        &self,
        _reader: &super::ObservationReaderContext,
        _query: &super::ObservationAncestryQueryV2,
        _now_ms: i64,
    ) -> Result<super::ObservationAncestryV2, ObservationQueryError> {
        Err(ObservationQueryError::Unavailable)
    }

    fn observed_catalog(
        &self,
        _reader: &super::ObservationReaderContext,
        _query: &super::ObservationCatalogQueryV2,
        _now_ms: i64,
    ) -> Result<super::ObservationCatalogPageV2, ObservationQueryError> {
        Err(ObservationQueryError::Unavailable)
    }

    fn observed_facts(
        &self,
        _reader: &super::ObservationReaderContext,
        _query: &super::ObservationFactsQueryV2,
        _now_ms: i64,
    ) -> Result<super::ObservationFactsPageV2, ObservationQueryError> {
        Err(ObservationQueryError::Unavailable)
    }

    fn observed_content_page(
        &self,
        _reader: &super::ObservationReaderContext,
        _query: &super::ObservationContentQueryV2,
        _now_ms: i64,
    ) -> Result<super::ObservationContentPageV2, ObservationQueryError> {
        Err(ObservationQueryError::Unavailable)
    }

    fn search_observed_text(
        &self,
        _reader: &super::ObservationReaderContext,
        _query: &super::ObservationSearchQueryV2,
        _now_ms: i64,
    ) -> Result<super::ObservationSearchPageV2, ObservationQueryError> {
        Err(ObservationQueryError::Unavailable)
    }

    fn observed_valuation(
        &self,
        _reader: &super::ObservationReaderContext,
        _request: &crate::LogicalRequestId,
        _now_ms: i64,
    ) -> Result<super::ObservationValuationSummaryV2, ObservationQueryError> {
        Err(ObservationQueryError::Unavailable)
    }

    fn observed_sessions(
        &self,
        _reader: &super::ObservationReaderContext,
        _query: &super::ObservationRequestQuery,
        _now_ms: i64,
    ) -> Result<super::ObservationSessionPageV2, ObservationQueryError> {
        Err(ObservationQueryError::Unavailable)
    }

    fn observed_timeline(
        &self,
        _reader: &super::ObservationReaderContext,
        _query: &super::ObservationRequestQuery,
        _now_ms: i64,
    ) -> Result<super::ObservationRequestPage, ObservationQueryError> {
        Err(ObservationQueryError::Unavailable)
    }

    fn observed_plan_quality_samples(
        &self,
        _reader: &super::ObservationReaderContext,
        _query: &super::PlanQualitySamplesQuery,
        _now_ms: i64,
    ) -> Result<super::PlanQualitySamplesPage, ObservationQueryError> {
        Err(ObservationQueryError::Unavailable)
    }

    /// V2 readers are admitted by Application; legacy adapters fail closed.
    fn list_observed_requests(
        &self,
        _reader: &super::ObservationReaderContext,
        _query: &super::ObservationRequestQuery,
        _now_ms: i64,
    ) -> Result<super::ObservationRequestPage, ObservationQueryError> {
        Err(ObservationQueryError::Unavailable)
    }

    fn list_sessions(
        &self,
        workspace_id: &WorkspaceId,
        query: &SessionListQueryV1,
    ) -> Result<SessionListV1, ObservationQueryError>;

    fn get_session(
        &self,
        workspace_id: &WorkspaceId,
        session_id: &SessionId,
        content_mode: ContentMode,
    ) -> Result<SessionDetailV1, ObservationQueryError>;

    fn get_receipt(
        &self,
        workspace_id: &WorkspaceId,
        receipt_id: &ReceiptId,
    ) -> Result<RoutingReceiptV1, ObservationQueryError>;

    fn get_value(
        &self,
        workspace_id: &WorkspaceId,
        query: &ValueQueryV1,
    ) -> Result<ValueViewV1, ObservationQueryError>;

    fn get_status(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<ObservationStatusV1, ObservationQueryError>;
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeletionDataClass {
    ContentOnly,
    FactsAndContent,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionDeletionSpecV1 {
    pub workspace_id: WorkspaceId,
    pub session_id: SessionId,
    pub data_class: DeletionDataClass,
    pub delete_rollups: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionDeletionPreviewV1 {
    pub spec: SessionDeletionSpecV1,
    pub store_revision: u64,
    pub change_digest: CanonicalDigest,
    pub content_instances: u64,
    pub turns: u64,
    pub requests: u64,
    pub receipts: u64,
    pub value_entries: u64,
    pub rollup_contributions: u64,
    pub tombstones: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionDeletionOutcomeV1 {
    pub new_store_revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tombstone_reason: Option<TombstoneReason>,
    pub garbage_collected_blobs: u64,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ObservationQueryError {
    #[error("observation query is not authorized")]
    Unauthorized,
    #[error("observation resource was not found")]
    NotFound,
    #[error("observation query is invalid")]
    InvalidQuery,
    #[error("observation store is unavailable")]
    Unavailable,
    #[error("observation store is corrupt or incompatible")]
    Corrupt,
    #[error("observation deletion preview is stale")]
    StalePreview,
    #[error("observation store revision conflicts with the reviewed change")]
    RevisionConflict,
}
