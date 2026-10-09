//! Finite observation intents over the existing Local Control operations.
pub use hiroute_domain::{
    ObservationAncestryQueryV2, ObservationAncestryV2, ObservationCatalogPageV2,
    ObservationCatalogQueryV2, ObservationContentPageV2, ObservationContentQueryV2,
    ObservationFactsPageV2, ObservationFactsQueryV2, ObservationRequestPage,
    ObservationRequestQuery, ObservationSearchPageV2, ObservationSearchQueryV2,
    ObservationSessionCorrelationKindV1, ObservationSessionPageV2, ObservationSessionSummaryV2,
    ObservationValueQueryV2, ObservationValueSummaryV2, PlanCompetenceFilter,
    PlanQualitySamplesPage, PlanQualitySamplesQuery,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationReadRequestV2 {
    pub schema: ObservationReadSchemaV2,
    pub intent: ObservationReadIntentV2,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum ObservationReadSchemaV2 {
    #[serde(rename = "hiroute.observation.query/v2")]
    V2,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "view",
    content = "query",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ObservationReadIntentV2 {
    Status(ObservationStatusQueryV2),
    Sessions(ObservationRequestQuery),
    Timeline(ObservationRequestQuery),
    Catalog(ObservationCatalogQueryV2),
    Ancestry(ObservationAncestryQueryV2),
    Content(ObservationContentQueryV2),
    Facts(ObservationFactsQueryV2),
    Search(ObservationSearchQueryV2),
    Value(ObservationValueQueryV2),
    HomeValue(ObservationHomeValueQueryV2),
    ValueReport(ObservationValueReportOptionsV2),
    PlanQuality(PlanQualitySamplesQuery),
}
impl ObservationReadRequestV2 {
    pub fn new(intent: ObservationReadIntentV2) -> Self {
        Self {
            schema: ObservationReadSchemaV2::V2,
            intent,
        }
    }
    pub fn operation(&self) -> &'static str {
        match self.intent {
            ObservationReadIntentV2::Status(_)
            | ObservationReadIntentV2::Sessions(_)
            | ObservationReadIntentV2::Search(_) => "ListSessions",
            ObservationReadIntentV2::PlanQuality(_) => "GetPlanQualitySamples",
            ObservationReadIntentV2::Facts(_) => "GetRoutingReceipt",
            ObservationReadIntentV2::Value(_)
            | ObservationReadIntentV2::HomeValue(_)
            | ObservationReadIntentV2::ValueReport(_) => "GetValue",
            _ => "GetSession",
        }
    }
    pub fn protected_operation(&self) -> &'static str {
        match self.intent {
            ObservationReadIntentV2::Status(_) | ObservationReadIntentV2::Sessions(_) => {
                "ListSessionsV2"
            }
            ObservationReadIntentV2::Timeline(_) => "GetSessionTimelineV2",
            ObservationReadIntentV2::Catalog(_) => "ReadSessionCatalogV2",
            ObservationReadIntentV2::Ancestry(_) => "ReadSessionAncestryV2",
            ObservationReadIntentV2::Content(_) => "ReadSessionContentV2",
            ObservationReadIntentV2::Facts(_) => "ReadSessionFactsV2",
            ObservationReadIntentV2::Search(_) => "SearchSessionContentV2",
            ObservationReadIntentV2::Value(_)
            | ObservationReadIntentV2::HomeValue(_)
            | ObservationReadIntentV2::ValueReport(_) => "GetValueV2",
            ObservationReadIntentV2::PlanQuality(_) => "ReadPlanQualitySamplesV1",
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

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationHomeValueQueryV2 {
    #[serde(default)]
    pub period: Option<crate::ValuePeriodV1>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub currency: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationStatusQueryV2 {}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
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
