//! Bounded non-text routing evidence, suitable for the existing safe-fact projection.
use crate::CanonicalDigest;
use serde::{Deserialize, Serialize};
use serde_json::Number;

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplicabilityStateV1 {
    NotRequested,
    Valid,
    Unavailable,
}
#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplicabilityReasonV1 {
    NoTarget,
    PartialTarget,
    Pending,
    DispatchNotStarted,
    Answered,
    InvalidOutput,
    Timeout,
    Unavailable,
    RejectedInput,
    AuthenticationRejected,
    RateLimited,
    EndpointRejected,
    RequiredQuestionMissing,
}
#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoricalProtectionActionV1 {
    NotConsidered,
    NotNeeded,
    Applied,
    Suppressed,
    RetainedUnavailable,
}
#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtectionEligibilityV1 {
    InvalidBoundary,
    NoTarget,
    PartialTarget,
    Unrated,
    DifferentPlan,
    DifferentCategory,
    DifferentPolicy,
    AmbiguousAttribution,
    SingleGroup,
    Eligible,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingObservationV1 {
    pub protocol: String,
    pub content_digest: CanonicalDigest,
    pub bundle_digest: CanonicalDigest,
    pub provider: String,
    pub model: String,
    pub standard: String,
    pub target_segment: Option<String>,
    pub target_complete: bool,
    pub eligibility: ProtectionEligibilityV1,
    pub applicability_requested: bool,
    pub applicability_state: ApplicabilityStateV1,
    pub applicability_reason: ApplicabilityReasonV1,
    pub applicability_probability: Option<Number>,
    pub protection_action: HistoricalProtectionActionV1,
    pub simple_probability: Option<Number>,
    pub simple_threshold_millis: u16,
    pub score: Option<Number>,
    pub competence_floor_millis: u16,
    pub applicability_boundary_millis: u16,
    pub request_bytes: u32,
    pub prior_retained: u32,
    pub prior_removed: u32,
    pub stage_retained: u32,
    pub stage_removed: u32,
}

impl RoutingObservationV1 {
    pub fn not_dispatched(p: &super::FrozenNativeProtocolV1, scope: &str) -> Self {
        let settings = p.scopes.get(scope);
        Self {
            protocol: p.protocol.schema.clone(),
            content_digest: p.digest.clone(),
            bundle_digest: CanonicalDigest::of(&p.protocol.fixed).expect("validated bundle"),
            provider: "unknown".into(),
            model: "unknown".into(),
            standard: settings.map_or("custom", |s| s.standard.as_str()).into(),
            target_segment: None,
            target_complete: false,
            eligibility: ProtectionEligibilityV1::Unrated,
            applicability_requested: false,
            applicability_state: ApplicabilityStateV1::NotRequested,
            applicability_reason: ApplicabilityReasonV1::DispatchNotStarted,
            applicability_probability: None,
            protection_action: HistoricalProtectionActionV1::NotConsidered,
            simple_probability: None,
            simple_threshold_millis: settings.map_or(800, |s| s.simple_threshold_millis),
            score: None,
            competence_floor_millis: settings.map_or(500, |s| s.competence_floor_millis),
            applicability_boundary_millis: 500,
            request_bytes: 0,
            prior_retained: 0,
            prior_removed: 0,
            stage_retained: 0,
            stage_removed: 0,
        }
    }
    pub fn validate(&self) -> bool {
        let probability = |p: &Number| {
            p.as_f64()
                .is_some_and(|p| p.is_finite() && (0.0..=1.0).contains(&p))
        };
        let text =
            |t: &str, max| !t.is_empty() && t.len() <= max && !t.chars().any(char::is_control);
        let valid = self.applicability_state == ApplicabilityStateV1::Valid;
        let low = self
            .score
            .as_ref()
            .and_then(Number::as_f64)
            .is_some_and(|s| s < f64::from(self.competence_floor_millis) / 1000.0);
        matches!(
            self.protocol.as_str(),
            super::SYSTEM_ONE_C_V1 | super::SYSTEM_ONE_C_V2
        ) && CanonicalDigest::parse(self.content_digest.as_str()).is_ok()
            && CanonicalDigest::parse(self.bundle_digest.as_str()).is_ok()
            && text(&self.provider, 128)
            && text(&self.model, 256)
            && matches!(self.standard.as_str(), "default" | "custom")
            && self.target_segment.as_ref().is_none_or(|s| text(s, 256))
            && self.simple_threshold_millis <= 1000
            && self.competence_floor_millis <= 1000
            && self.applicability_boundary_millis == 500
            && self.request_bytes <= 262_144
            && self.prior_retained <= 8
            && self.applicability_probability.is_some() == valid
            && self
                .applicability_probability
                .iter()
                .chain(self.simple_probability.iter())
                .chain(self.score.iter())
                .all(probability)
            && (!valid || self.applicability_requested)
            && (self.applicability_state != ApplicabilityStateV1::Unavailable
                || self.applicability_requested)
            && (self.applicability_state != ApplicabilityStateV1::NotRequested
                || !self.applicability_requested)
            && (!matches!(
                self.protection_action,
                HistoricalProtectionActionV1::Applied
                    | HistoricalProtectionActionV1::Suppressed
                    | HistoricalProtectionActionV1::RetainedUnavailable
            ) || (self.eligibility == ProtectionEligibilityV1::Eligible
                && self.target_complete
                && low
                && self.simple_probability.is_some()))
            && (self.protection_action != HistoricalProtectionActionV1::Suppressed
                || self
                    .applicability_probability
                    .as_ref()
                    .and_then(Number::as_f64)
                    .is_some_and(|a| a < 0.5))
            && (self.protection_action != HistoricalProtectionActionV1::Applied
                || self
                    .applicability_probability
                    .as_ref()
                    .and_then(Number::as_f64)
                    .is_some_and(|a| a >= 0.5))
            && (self.protection_action != HistoricalProtectionActionV1::RetainedUnavailable
                || !valid)
    }
}
