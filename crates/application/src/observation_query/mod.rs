//! Authorization boundary for Session/Turn/Search/Receipt/Value queries.
//!
//! The adapter port returns stored facts. This service never reads current Plan/price state and
//! never manufactures content or a natural-language explanation.

use std::sync::Arc;

use hiroute_domain::{
    ContentMode, ObservationCapabilityV1, ObservationPrincipalV1, ObservationQueryError,
    ObservationQueryPort, ObservationStatusV1, ReceiptId, RoutingReceiptV1, SessionDetailV1,
    SessionId, SessionListQueryV1, SessionListV1, ValueQueryV1, ValueViewV1,
};

pub(crate) mod control;

#[derive(Clone)]
pub struct ObservationQueryService {
    port: Arc<dyn ObservationQueryPort>,
}

impl ObservationQueryService {
    pub fn preview_session_deletion_v2(
        &self,
        principal: &ObservationPrincipalV1,
        spec: &hiroute_domain::SessionDeletionSpecV1,
        through_ms: i64,
    ) -> Result<hiroute_domain::SessionDeletionPreviewV2, ObservationQueryError> {
        require(principal, ObservationCapabilityV1::ManageRetention)?;
        self.port
            .preview_session_deletion_v2(principal, spec, through_ms)
    }
    pub fn apply_session_deletion_v2(
        &self,
        principal: &ObservationPrincipalV1,
        preview: &hiroute_domain::SessionDeletionPreviewV2,
        accepted: &hiroute_domain::CanonicalDigest,
        now_ms: i64,
    ) -> Result<hiroute_domain::SessionDeletionOutcomeV2, ObservationQueryError> {
        require(principal, ObservationCapabilityV1::ManageRetention)?;
        self.port
            .apply_session_deletion_v2(principal, preview, accepted, now_ms)
    }

    pub fn observed_maintenance_status(
        &self,
        reader: &hiroute_domain::ObservationReaderContext,
        now_ms: i64,
    ) -> Result<hiroute_domain::ObservationMaintenanceStatusV2, ObservationQueryError> {
        reader.check(now_ms, false, false)?;
        self.port.observed_maintenance_status(reader, now_ms)
    }

    pub fn observed_value_report(
        &self,
        reader: &hiroute_domain::ObservationReaderContext,
        query: &hiroute_domain::ObservationValueQueryV2,
        group_by: hiroute_domain::ValueGroupByV1,
        now_ms: i64,
    ) -> Result<hiroute_domain::ObservationValueReportV2, ObservationQueryError> {
        reader.check(now_ms, false, false)?;
        self.port
            .observed_value_report(reader, query, group_by, now_ms)
    }

    pub fn observed_value_totals(
        &self,
        reader: &hiroute_domain::ObservationReaderContext,
        query: &hiroute_domain::ObservationValueQueryV2,
        now_ms: i64,
    ) -> Result<hiroute_domain::ObservationValueSummaryV2, ObservationQueryError> {
        reader.check(now_ms, false, false)?;
        self.port.observed_value_totals(reader, query, now_ms)
    }

    pub fn observed_ancestry(
        &self,
        reader: &hiroute_domain::ObservationReaderContext,
        query: &hiroute_domain::ObservationAncestryQueryV2,
        now_ms: i64,
    ) -> Result<hiroute_domain::ObservationAncestryV2, ObservationQueryError> {
        reader.check(now_ms, true, false)?;
        self.port.observed_ancestry(reader, query, now_ms)
    }

    pub fn observed_catalog(
        &self,
        reader: &hiroute_domain::ObservationReaderContext,
        query: &hiroute_domain::ObservationCatalogQueryV2,
        now_ms: i64,
    ) -> Result<hiroute_domain::ObservationCatalogPageV2, ObservationQueryError> {
        reader.check(now_ms, true, false)?;
        self.port.observed_catalog(reader, query, now_ms)
    }

    pub fn observed_timeline(
        &self,
        reader: &hiroute_domain::ObservationReaderContext,
        query: &hiroute_domain::ObservationRequestQuery,
        now_ms: i64,
    ) -> Result<hiroute_domain::ObservationRequestPage, ObservationQueryError> {
        reader.check(now_ms, false, false)?;
        self.port.observed_timeline(reader, query, now_ms)
    }

    pub fn observed_sessions(
        &self,
        reader: &hiroute_domain::ObservationReaderContext,
        query: &hiroute_domain::ObservationRequestQuery,
        now_ms: i64,
    ) -> Result<hiroute_domain::ObservationSessionPageV2, ObservationQueryError> {
        reader.check(now_ms, false, false)?;
        self.port.observed_sessions(reader, query, now_ms)
    }

    pub fn observed_plan_quality_samples(
        &self,
        reader: &hiroute_domain::ObservationReaderContext,
        query: &hiroute_domain::PlanQualitySamplesQuery,
        now_ms: i64,
    ) -> Result<hiroute_domain::PlanQualitySamplesPage, ObservationQueryError> {
        reader.check(now_ms, false, false)?;
        self.port
            .observed_plan_quality_samples(reader, query, now_ms)
    }

    pub fn new(port: Arc<dyn ObservationQueryPort>) -> Self {
        Self { port }
    }

    /// Only trusted admission may construct a reader. Check expiry on every page,
    /// including implementations whose backing adapter has not adopted V2 yet.
    pub fn list_observed_requests(
        &self,
        reader: &hiroute_domain::ObservationReaderContext,
        query: &hiroute_domain::ObservationRequestQuery,
        now_ms: i64,
    ) -> Result<hiroute_domain::ObservationRequestPage, ObservationQueryError> {
        reader.check(now_ms, false, false)?;
        self.port.list_observed_requests(reader, query, now_ms)
    }

    pub fn observed_valuation(
        &self,
        reader: &hiroute_domain::ObservationReaderContext,
        request: &hiroute_domain::LogicalRequestId,
        now_ms: i64,
    ) -> Result<hiroute_domain::ObservationValuationSummaryV2, ObservationQueryError> {
        reader.check(now_ms, false, false)?;
        self.port.observed_valuation(reader, request, now_ms)
    }

    pub fn search_observed_text(
        &self,
        reader: &hiroute_domain::ObservationReaderContext,
        query: &hiroute_domain::ObservationSearchQueryV2,
        now_ms: i64,
    ) -> Result<hiroute_domain::ObservationSearchPageV2, ObservationQueryError> {
        reader.check(now_ms, true, true)?;
        self.port.search_observed_text(reader, query, now_ms)
    }

    pub fn observed_content_page(
        &self,
        reader: &hiroute_domain::ObservationReaderContext,
        query: &hiroute_domain::ObservationContentQueryV2,
        now_ms: i64,
    ) -> Result<hiroute_domain::ObservationContentPageV2, ObservationQueryError> {
        reader.check(now_ms, true, false)?;
        self.port.observed_content_page(reader, query, now_ms)
    }

    pub fn observed_facts(
        &self,
        reader: &hiroute_domain::ObservationReaderContext,
        query: &hiroute_domain::ObservationFactsQueryV2,
        now_ms: i64,
    ) -> Result<hiroute_domain::ObservationFactsPageV2, ObservationQueryError> {
        reader.check(now_ms, false, false)?;
        self.port.observed_facts(reader, query, now_ms)
    }

    pub fn list_sessions(
        &self,
        principal: &ObservationPrincipalV1,
        query: &SessionListQueryV1,
    ) -> Result<SessionListV1, ObservationQueryError> {
        require(principal, ObservationCapabilityV1::ReadFacts)?;
        if query.query.is_some() {
            require(principal, ObservationCapabilityV1::SearchContent)?;
        }
        self.port.list_sessions(&principal.workspace_id, query)
    }

    pub fn get_session(
        &self,
        principal: &ObservationPrincipalV1,
        session_id: &SessionId,
        content_mode: ContentMode,
    ) -> Result<SessionDetailV1, ObservationQueryError> {
        require(principal, ObservationCapabilityV1::ReadFacts)?;
        if content_mode != ContentMode::None {
            require(principal, ObservationCapabilityV1::ReadContent)?;
        }
        self.port
            .get_session(&principal.workspace_id, session_id, content_mode)
    }

    pub fn get_receipt(
        &self,
        principal: &ObservationPrincipalV1,
        receipt_id: &ReceiptId,
    ) -> Result<RoutingReceiptV1, ObservationQueryError> {
        require(principal, ObservationCapabilityV1::ReadFacts)?;
        // Legacy receipts can contain literal matching evidence, so the raw DTO
        // is content-bearing even if a UI chooses to collapse its facts panel.
        require(principal, ObservationCapabilityV1::ReadContent)?;
        self.port.get_receipt(&principal.workspace_id, receipt_id)
    }

    pub fn get_value(
        &self,
        principal: &ObservationPrincipalV1,
        query: &ValueQueryV1,
    ) -> Result<ValueViewV1, ObservationQueryError> {
        require(principal, ObservationCapabilityV1::ReadFacts)?;
        self.port.get_value(&principal.workspace_id, query)
    }

    pub fn get_status(
        &self,
        principal: &ObservationPrincipalV1,
    ) -> Result<ObservationStatusV1, ObservationQueryError> {
        require(principal, ObservationCapabilityV1::ReadFacts)?;
        self.port.get_status(&principal.workspace_id)
    }
}

fn require(
    principal: &ObservationPrincipalV1,
    capability: ObservationCapabilityV1,
) -> Result<(), ObservationQueryError> {
    if principal.allows(capability) {
        Ok(())
    } else {
        Err(ObservationQueryError::Unauthorized)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use hiroute_domain::{
        ContentAccess, ContentCompleteness, CorrelationProvenance, FactsCompleteness,
        ObservationCompletenessScope, ObservationStatusV1, RetentionPolicyV1, SessionSummaryV1,
        ValueViewV1, WorkspaceId,
    };

    use super::*;

    struct FakeQuery {
        calls: AtomicUsize,
    }

    impl FakeQuery {
        fn new() -> Self {
            Self {
                calls: AtomicUsize::new(0),
            }
        }
    }

    impl ObservationQueryPort for FakeQuery {
        fn list_sessions(
            &self,
            _workspace_id: &WorkspaceId,
            _query: &SessionListQueryV1,
        ) -> Result<SessionListV1, ObservationQueryError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(SessionListV1 { sessions: vec![] })
        }

        fn get_session(
            &self,
            _workspace_id: &WorkspaceId,
            session_id: &SessionId,
            content_mode: ContentMode,
        ) -> Result<SessionDetailV1, ObservationQueryError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(SessionDetailV1 {
                summary: SessionSummaryV1 {
                    session_id: session_id.clone(),
                    agent_id: "agent".to_owned(),
                    correlation_provenance: CorrelationProvenance::AgentSupplied,
                    started_at_ms: 1,
                    updated_at_ms: 1,
                    retention_deadline_ms: 1 + hiroute_domain::SEVEN_DAYS_MILLIS,
                    turn_count: 0,
                    request_count: 0,
                    model_switch: None,
                    facts_completeness: FactsCompleteness::Complete,
                    content_completeness: ContentCompleteness::Unknown,
                    completeness_scope: ObservationCompletenessScope::GatewayVisible,
                    tombstone_reason: None,
                },
                turns: vec![],
                content_access: if content_mode == ContentMode::None {
                    ContentAccess::NotRequested
                } else {
                    ContentAccess::Authorized
                },
            })
        }

        fn get_receipt(
            &self,
            _workspace_id: &WorkspaceId,
            _receipt_id: &ReceiptId,
        ) -> Result<RoutingReceiptV1, ObservationQueryError> {
            unreachable!("not needed by authorization tests")
        }

        fn get_value(
            &self,
            _workspace_id: &WorkspaceId,
            query: &ValueQueryV1,
        ) -> Result<ValueViewV1, ObservationQueryError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(ValueViewV1 {
                value_calculation_basis: hiroute_domain::VALUE_CALCULATION_BASIS_V1.to_owned(),
                agent_plan_id: query.agent_plan_id.clone(),
                currency: query.currency.clone(),
                group_by: query.group_by,
                groups: vec![],
                entries: vec![],
                daily_aggregates: vec![],
                input_tokens: 0,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                reasoning_tokens: 0,
                baseline_api_equivalent_cost_micros: None,
                chosen_api_equivalent_cost_micros: None,
                actual_incremental_cost_micros: None,
                routing_savings_micros: None,
                entitlement_savings_micros: None,
                estimated_total_savings_micros: None,
                price_version_refs: Default::default(),
                price_override_revision_refs: Default::default(),
                facts_completeness: FactsCompleteness::Unknown,
                detail_available: true,
            })
        }

        fn get_status(
            &self,
            _workspace_id: &WorkspaceId,
        ) -> Result<ObservationStatusV1, ObservationQueryError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(ObservationStatusV1 {
                retention: RetentionPolicyV1::default(),
                store_revision: 0,
                facts_completeness: FactsCompleteness::Complete,
                content_completeness: ContentCompleteness::Unknown,
                completeness_scope: ObservationCompletenessScope::GatewayVisible,
                gaps: vec![],
                gaps_truncated: false,
                activity_bytes: 0,
                content_bytes: 0,
                production_exporter: "not_installed".to_owned(),
            })
        }
    }

    #[test]
    fn observation_query_facts_only_principal_cannot_read_content() {
        let port = Arc::new(FakeQuery::new());
        let service = ObservationQueryService::new(port.clone());
        let principal = ObservationPrincipalV1::facts_only(WorkspaceId::default());
        let result = service.get_session(
            &principal,
            &SessionId::parse("session-1").unwrap(),
            ContentMode::Messages,
        );
        assert_eq!(result.unwrap_err(), ObservationQueryError::Unauthorized);
        assert_eq!(port.calls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn observation_query_list_never_requires_content_without_search() {
        let port = Arc::new(FakeQuery::new());
        let service = ObservationQueryService::new(port.clone());
        let principal = ObservationPrincipalV1::facts_only(WorkspaceId::default());
        assert!(
            service
                .list_sessions(&principal, &SessionListQueryV1::default())
                .is_ok()
        );
        assert_eq!(port.calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn observation_query_search_requires_independent_capability() {
        let port = Arc::new(FakeQuery::new());
        let service = ObservationQueryService::new(port.clone());
        let principal = ObservationPrincipalV1::facts_only(WorkspaceId::default());
        let query = SessionListQueryV1 {
            query: Some("needle".to_owned()),
            ..SessionListQueryV1::default()
        };
        assert_eq!(
            service.list_sessions(&principal, &query).unwrap_err(),
            ObservationQueryError::Unauthorized
        );
        assert_eq!(port.calls.load(Ordering::Relaxed), 0);
    }
}
