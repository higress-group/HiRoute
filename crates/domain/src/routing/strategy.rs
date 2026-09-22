use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{AgentPlanDisplayName, AgentPlanPurpose};

use super::{AGENT_PLAN_DESIRED_SCHEMA_V1, ReasoningSelectionV1};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPlanDesiredV1 {
    pub schema: String,
    pub display_name: AgentPlanDisplayName,
    pub purpose: AgentPlanPurpose,
    pub requirements: CapabilityRequirementsV1,
    pub limits: RoutingLimitsV1,
    pub strategy: AgentPlanStrategyV1,
}

impl AgentPlanDesiredV1 {
    pub fn validate(&self) -> Result<(), RoutingStrategyError> {
        if self.schema != AGENT_PLAN_DESIRED_SCHEMA_V1 {
            return Err(RoutingStrategyError::UnsupportedSchema);
        }
        self.display_name
            .validate()
            .map_err(|_| RoutingStrategyError::InvalidMetadata)?;
        self.purpose
            .validate()
            .map_err(|_| RoutingStrategyError::InvalidMetadata)?;
        self.requirements.validate()?;
        self.limits.validate()?;
        self.strategy.validate()
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityRequirementsV1 {
    #[serde(default)]
    pub tool: bool,
    #[serde(default)]
    pub vision: bool,
    #[serde(default)]
    pub streaming: bool,
    #[serde(default)]
    pub minimum_context_tokens: u64,
    #[serde(default)]
    pub minimum_output_tokens: u64,
}

impl CapabilityRequirementsV1 {
    pub fn validate(&self) -> Result<(), RoutingStrategyError> {
        if self.minimum_context_tokens > 10_000_000 || self.minimum_output_tokens > 10_000_000 {
            Err(RoutingStrategyError::InvalidRequirements)
        } else {
            Ok(())
        }
    }
}

pub const DEFAULT_PLAN_CONTEXT_WINDOW_TOKENS: u64 = 272_000;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingLimitsV1 {
    /// None follows the bounded default; omission preserves historical signed digests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window_tokens: Option<u64>,
    pub maximum_attempts: u16,
    pub request_timeout_ms: u64,
    pub attempt_timeout_ms: u64,
}

impl RoutingLimitsV1 {
    pub fn validate(&self) -> Result<(), RoutingStrategyError> {
        if self
            .context_window_tokens
            .is_some_and(|value| value == 0 || value > i64::MAX as u64)
            || !(1..=64).contains(&self.maximum_attempts)
            || !(1_000..=3_600_000).contains(&self.request_timeout_ms)
            || !(1_000..=self.request_timeout_ms).contains(&self.attempt_timeout_ms)
        {
            Err(RoutingStrategyError::InvalidLimits)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentPlanStrategyV1 {
    SmartSaving {
        economy_candidates: Vec<CandidateSelectionV1>,
        quality_anchor_binding_id: String,
        primary_candidates: Vec<CandidateSelectionV1>,
        quality_guard_score_gap_tenths: u8,
        #[serde(default)]
        complex_keywords: Vec<String>,
    },
    FreeFirst {
        free_pool: FreePoolSpecV1,
        fallback_policy: FreeFallbackPolicy,
        #[serde(default)]
        primary_candidates: Vec<CandidateSelectionV1>,
    },
    Custom {
        candidates: Vec<CandidateSelectionV1>,
    },
}

impl AgentPlanStrategyV1 {
    pub fn validate(&self) -> Result<(), RoutingStrategyError> {
        match self {
            Self::SmartSaving {
                economy_candidates,
                quality_anchor_binding_id,
                primary_candidates,
                quality_guard_score_gap_tenths,
                complex_keywords,
            } => {
                validate_candidates(economy_candidates)?;
                validate_candidates(primary_candidates)?;
                if !valid_reference(quality_anchor_binding_id)
                    || !economy_candidates
                        .iter()
                        .any(|candidate| candidate.binding_id == *quality_anchor_binding_id)
                {
                    return Err(RoutingStrategyError::InvalidQualityAnchor);
                }
                if *quality_guard_score_gap_tenths > 45 {
                    return Err(RoutingStrategyError::InvalidQualityGuard);
                }
                validate_keywords(complex_keywords)
            }
            Self::FreeFirst {
                free_pool,
                fallback_policy,
                primary_candidates,
            } => {
                free_pool.validate()?;
                match fallback_policy {
                    FreeFallbackPolicy::FreeOnly if !primary_candidates.is_empty() => {
                        Err(RoutingStrategyError::StrictFreeHasPrimary)
                    }
                    FreeFallbackPolicy::PrimaryFallback => validate_candidates(primary_candidates),
                    FreeFallbackPolicy::FreeOnly => Ok(()),
                }
            }
            Self::Custom { candidates } => validate_candidates(candidates),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FreePoolMode {
    AutomaticAllAvailable,
    Manual,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FreeFallbackPolicy {
    FreeOnly,
    PrimaryFallback,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FreePoolSpecV1 {
    pub mode: FreePoolMode,
    /// Used only in manual mode. Array order is product semantics.
    #[serde(default)]
    pub candidates: Vec<CandidateSelectionV1>,
    /// Optional exact choices keyed by Binding for automatic candidates. Toggle and budget
    /// candidates without a choice are excluded with a typed reason by the compiler.
    #[serde(default)]
    pub automatic_reasoning: BTreeMap<String, ReasoningSelectionV1>,
}

impl FreePoolSpecV1 {
    pub fn validate(&self) -> Result<(), RoutingStrategyError> {
        match self.mode {
            FreePoolMode::AutomaticAllAvailable if !self.candidates.is_empty() => {
                return Err(RoutingStrategyError::AutomaticPoolHasManualCandidates);
            }
            FreePoolMode::Manual => {
                if self.candidates.is_empty() {
                    return Err(RoutingStrategyError::NoFreeCandidates);
                }
                validate_candidates(&self.candidates)?;
                if !self.automatic_reasoning.is_empty() {
                    return Err(RoutingStrategyError::ManualPoolHasAutomaticChoices);
                }
            }
            FreePoolMode::AutomaticAllAvailable => {}
        }
        if self
            .automatic_reasoning
            .keys()
            .any(|key| !valid_reference(key))
        {
            return Err(RoutingStrategyError::InvalidCandidateReference);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateSelectionV1 {
    pub binding_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningSelectionV1>,
}

pub(super) fn validate_candidates(
    candidates: &[CandidateSelectionV1],
) -> Result<(), RoutingStrategyError> {
    if candidates.is_empty() || candidates.len() > 128 {
        return Err(RoutingStrategyError::InvalidCandidateCount);
    }
    let mut seen = std::collections::BTreeSet::new();
    for candidate in candidates {
        if !valid_reference(&candidate.binding_id) || !seen.insert(&candidate.binding_id) {
            return Err(RoutingStrategyError::InvalidCandidateReference);
        }
    }
    Ok(())
}

pub(super) fn validate_keywords(keywords: &[String]) -> Result<(), RoutingStrategyError> {
    if keywords.len() > 64 {
        return Err(RoutingStrategyError::InvalidKeyword);
    }
    let mut seen = std::collections::BTreeSet::new();
    for keyword in keywords {
        let folded = keyword.to_lowercase();
        if !(1..=64).contains(&keyword.chars().count())
            || keyword.trim() != keyword
            || keyword.chars().any(char::is_control)
            || !seen.insert(folded)
        {
            return Err(RoutingStrategyError::InvalidKeyword);
        }
    }
    Ok(())
}

pub(crate) fn valid_reference(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.contains("//")
        && !value.contains("..")
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/' | b':')
        })
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RoutingStrategyError {
    #[error("AgentPlan desired schema is unsupported")]
    UnsupportedSchema,
    #[error("AgentPlan display name or purpose is invalid")]
    InvalidMetadata,
    #[error("capability requirements are invalid")]
    InvalidRequirements,
    #[error("routing limits are invalid")]
    InvalidLimits,
    #[error("candidate list must be non-empty and bounded")]
    InvalidCandidateCount,
    #[error("candidate reference is invalid or duplicated")]
    InvalidCandidateReference,
    #[error("quality guard is outside the rating scale")]
    InvalidQualityGuard,
    #[error("quality anchor must name one exact economy candidate")]
    InvalidQualityAnchor,
    #[error("complex keyword is invalid or duplicated")]
    InvalidKeyword,
    #[error("automatic free pool cannot carry a manual candidate list")]
    AutomaticPoolHasManualCandidates,
    #[error("manual free pool cannot carry automatic reasoning choices")]
    ManualPoolHasAutomaticChoices,
    #[error("strict-free routing cannot contain a primary group")]
    StrictFreeHasPrimary,
    #[error("NO_FREE_CANDIDATES")]
    NoFreeCandidates,
}

#[cfg(test)]
mod routing_strategy_tests {
    use super::*;

    #[test]
    fn routing_strict_free_rejects_a_hidden_primary_group() {
        let strategy = AgentPlanStrategyV1::FreeFirst {
            free_pool: FreePoolSpecV1 {
                mode: FreePoolMode::AutomaticAllAvailable,
                candidates: Vec::new(),
                automatic_reasoning: BTreeMap::new(),
            },
            fallback_policy: FreeFallbackPolicy::FreeOnly,
            primary_candidates: vec![CandidateSelectionV1 {
                binding_id: "binding/paid".into(),
                reasoning: None,
            }],
        };
        assert_eq!(
            strategy.validate().unwrap_err(),
            RoutingStrategyError::StrictFreeHasPrimary
        );
    }
}
