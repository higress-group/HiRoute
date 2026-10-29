//! Finite observation intents over the existing Local Control operations.
pub use hiroute_domain::{
    AnalysisCostBasisV1, AnalysisCostGroupV1, AnalysisPeriodV1, AnalysisViewV1,
    ObservationAnalysisPageV1, ObservationAnalysisQueryV1, ObservationAncestryQueryV2,
    ObservationAncestryV2, ObservationCatalogPageV2, ObservationCatalogQueryV2,
    ObservationContentPageV2, ObservationContentQueryV2, ObservationFactsPageV2,
    ObservationFactsQueryV2, ObservationRequestPage, ObservationRequestQuery,
    ObservationSearchPageV2, ObservationSearchQueryV2, ObservationSessionCorrelationKindV1,
    ObservationSessionPageV2, ObservationSessionSearchPageV1, ObservationSessionSummaryV2,
    ObservationValueQueryV2, ObservationValueSummaryV2, PlanCompetenceFilter,
    PlanQualitySamplesPage, PlanQualitySamplesQuery, PlanTrustQueryV1, PlanTrustReplayQueryV1,
    PriceValuationKindV1, RoutingTimelinePageV1, RoutingTimelineQueryV1, TrustGroupByV1,
    TrustRevisionNameV1, TrustRevisionV1,
};
use serde::{Deserialize, Serialize};
#[derive(schemars::JsonSchema, Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationReadFailureReasonV1 {
    Timeout,
    ReaderBusy,
    IndexUnavailable,
    StorageUnavailable,
}

#[derive(schemars::JsonSchema, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationReadFailureV1 {
    pub reason: ObservationReadFailureReasonV1,
    pub retry_after_ms: u32,
    pub action: String,
}
#[derive(schemars::JsonSchema, Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationReadRequestV2 {
    pub schema: ObservationReadSchemaV2,
    pub intent: ObservationReadIntentV2,
    /// Opt into typed bounded-read errors. Omitted/false preserves released errors
    /// and the canonical request bytes used by existing grants and cursors.
    #[serde(default, skip_serializing_if = "is_false")]
    pub read_failure_details: bool,
}
fn is_false(value: &bool) -> bool {
    !value
}
#[derive(schemars::JsonSchema, Clone, Copy, Debug, Serialize, Deserialize)]
pub enum ObservationReadSchemaV2 {
    #[serde(rename = "hiroute.observation.query/v2")]
    V2,
}
#[derive(schemars::JsonSchema, Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "view",
    content = "query",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ObservationReadIntentV2 {
    Analysis(ObservationAnalysisQueryV1),
    Status(ObservationStatusQueryV2),
    Sessions(ObservationRequestQuery),
    Timeline(ObservationRequestQuery),
    RoutingTimeline(RoutingTimelineQueryV1),
    Catalog(ObservationCatalogQueryV2),
    Ancestry(ObservationAncestryQueryV2),
    Content(ObservationContentQueryV2),
    Facts(ObservationFactsQueryV2),
    Search(ObservationSearchQueryV2),
    SessionSearch(ObservationSearchQueryV2),
    Value(ObservationValueQueryV2),
    HomeValue(ObservationHomeValueQueryV2),
    ValueReport(ObservationValueReportOptionsV2),
    ApiEquivalentValue(ApiEquivalentValueOptionsV1),
    PlanQuality(Box<PlanQualitySamplesQuery>),
    PlanTrust(PlanTrustQueryV1),
    PlanTrustReplay(PlanTrustReplayQueryV1),
    PlanTrustSuggestions(ObservationStatusQueryV2),
}
impl ObservationReadRequestV2 {
    pub fn new(intent: ObservationReadIntentV2) -> Self {
        Self {
            schema: ObservationReadSchemaV2::V2,
            intent,
            read_failure_details: false,
        }
    }
    pub fn operation(&self) -> &'static str {
        match self.intent {
            ObservationReadIntentV2::Analysis(ref q) => match q.view {
                AnalysisViewV1::Conversations | AnalysisViewV1::Switches => "ListSessions",
                AnalysisViewV1::CostRanking | AnalysisViewV1::SavingsSummary => "GetValue",
                _ => "GetSession",
            },
            ObservationReadIntentV2::Status(_)
            | ObservationReadIntentV2::Sessions(_)
            | ObservationReadIntentV2::Search(_)
            | ObservationReadIntentV2::SessionSearch(_) => "ListSessions",
            ObservationReadIntentV2::RoutingTimeline(_) => "GetRoutingTimeline",
            ObservationReadIntentV2::PlanQuality(_) => "GetPlanQualitySamples",
            ObservationReadIntentV2::PlanTrust(_) => "GetPlanTrust",
            ObservationReadIntentV2::PlanTrustReplay(_) => "GetPlanTrustReplay",
            ObservationReadIntentV2::PlanTrustSuggestions(_) => "GetPlanTrustSuggestions",
            ObservationReadIntentV2::Facts(_) => "GetRoutingReceipt",
            ObservationReadIntentV2::Value(_)
            | ObservationReadIntentV2::HomeValue(_)
            | ObservationReadIntentV2::ValueReport(_)
            | ObservationReadIntentV2::ApiEquivalentValue(_) => "GetValue",
            _ => "GetSession",
        }
    }
    pub fn protected_operation(&self) -> &'static str {
        match self.intent {
            ObservationReadIntentV2::Analysis(ref q) => match q.view {
                AnalysisViewV1::Conversations | AnalysisViewV1::Switches => "ListSessionsV2",
                AnalysisViewV1::CostRanking | AnalysisViewV1::SavingsSummary => "GetValueV2",
                _ => "GetSessionTimelineV2",
            },
            ObservationReadIntentV2::Status(_) | ObservationReadIntentV2::Sessions(_) => {
                "ListSessionsV2"
            }
            ObservationReadIntentV2::Timeline(_) => "GetSessionTimelineV2",
            ObservationReadIntentV2::RoutingTimeline(_) => "ReadRoutingTimelineV1",
            ObservationReadIntentV2::Catalog(_) => "ReadSessionCatalogV2",
            ObservationReadIntentV2::Ancestry(_) => "ReadSessionAncestryV2",
            ObservationReadIntentV2::Content(_) => "ReadSessionContentV2",
            ObservationReadIntentV2::Facts(_) => "ReadSessionFactsV2",
            ObservationReadIntentV2::Search(_) | ObservationReadIntentV2::SessionSearch(_) => {
                "SearchSessionContentV2"
            }
            ObservationReadIntentV2::Value(_)
            | ObservationReadIntentV2::HomeValue(_)
            | ObservationReadIntentV2::ValueReport(_)
            | ObservationReadIntentV2::ApiEquivalentValue(_) => "GetValueV2",
            ObservationReadIntentV2::PlanQuality(_) => "ReadPlanQualitySamplesV1",
            ObservationReadIntentV2::PlanTrust(_) => "ReadPlanTrustV1",
            ObservationReadIntentV2::PlanTrustReplay(_) => "ReadPlanTrustReplayV1",
            ObservationReadIntentV2::PlanTrustSuggestions(_) => "ReadPlanTrustSuggestionsV1",
        }
    }
}

pub use hiroute_domain::{
    DeletionDataClass, SessionDeletionOutcomeV2, SessionDeletionPreviewV2, SessionDeletionSpecV1,
};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationDeletePreviewRequestV2 {
    pub spec: SessionDeletionSpecV1,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationDeleteApplyRequestV2 {
    pub preview: SessionDeletionPreviewV2,
    pub accepted_digest: hiroute_domain::CanonicalDigest,
}

#[derive(schemars::JsonSchema, Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationHomeValueQueryV2 {
    #[serde(default)]
    pub period: Option<crate::ValuePeriodV1>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub currency: Option<String>,
}

#[derive(schemars::JsonSchema, Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationStatusQueryV2 {}

#[derive(schemars::JsonSchema, Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationValueReportOptionsV2 {
    pub agent_plan_id: Option<crate::AgentPlanId>,
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    pub period: Option<crate::ValuePeriodV1>,
    #[serde(default)]
    pub group_by: crate::ValueGroupByV1,
    pub currency: Option<String>,
    pub session_id: Option<crate::SessionId>,
}

/// Opt-in current-price estimate; old value-report calls keep their output.
#[derive(schemars::JsonSchema, Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiEquivalentValueOptionsV1 {
    pub report: ObservationValueReportOptionsV2,
    pub reference_binding_id: String,
}

/// Explicit null disables early capacity expiry; an omitted value is invalid.
#[derive(schemars::JsonSchema, Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationCapacityUpdateV1 {
    #[serde(deserialize_with = "deserialize_capacity_limit")]
    #[schemars(required)]
    pub max_active_bytes: Option<u64>,
}
fn deserialize_capacity_limit<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<u64>, D::Error> {
    let value = Option::<u64>::deserialize(d)?;
    if value == Some(0) {
        return Err(serde::de::Error::custom(
            "capacity must be positive or null",
        ));
    }
    Ok(value)
}

#[cfg(test)]
mod capacity_tests {
    use super::*;
    #[test]
    fn capacity_requires_explicit_positive_integer_or_null() {
        for input in [
            "{}",
            "{\"max_active_bytes\":0}",
            "{\"max_active_bytes\":-1}",
            "{\"max_active_bytes\":18446744073709551616}",
            "{\"max_active_bytes\":1.5}",
            "{\"max_active_bytes\":\"20G\"}",
            "{\"max_active_bytes\":20,\"extra\":true}",
        ] {
            assert!(
                serde_json::from_str::<ObservationCapacityUpdateV1>(input).is_err(),
                "{input}"
            );
        }
        assert_eq!(
            serde_json::from_str::<ObservationCapacityUpdateV1>("{\"max_active_bytes\":null}")
                .unwrap()
                .max_active_bytes,
            None
        );
        assert_eq!(
            serde_json::from_str::<ObservationCapacityUpdateV1>(
                "{\"max_active_bytes\":18446744073709551615}"
            )
            .unwrap()
            .max_active_bytes,
            Some(u64::MAX)
        );
    }
}

#[cfg(test)]
mod read_failure_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn detailed_errors_are_opt_in_without_changing_released_request_identity() {
        let released = json!({"schema":"hiroute.observation.query/v2",
            "intent":{"view":"status","query":{}}});
        let request: ObservationReadRequestV2 = serde_json::from_value(released.clone()).unwrap();
        assert!(!request.read_failure_details);
        assert_eq!(serde_json::to_value(&request).unwrap(), released);
        let old_digest = crate::CanonicalDigest::of(&released).unwrap();
        assert_eq!(crate::CanonicalDigest::of(&request).unwrap(), old_digest);
        let mut explicit_false = released.clone();
        explicit_false["read_failure_details"] = json!(false);
        let mut detailed: ObservationReadRequestV2 =
            serde_json::from_value(explicit_false).unwrap();
        assert_eq!(crate::CanonicalDigest::of(&detailed).unwrap(), old_digest);
        detailed.read_failure_details = true;
        assert_ne!(crate::CanonicalDigest::of(&detailed).unwrap(), old_digest);
        for invalid in [json!(null), json!("true"), json!(1)] {
            let mut wire = released.clone();
            wire["read_failure_details"] = invalid;
            assert!(serde_json::from_value::<ObservationReadRequestV2>(wire).is_err());
        }
    }
}
