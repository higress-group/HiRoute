//! Versioned explicit editor state. Parked modes are draft data, never execution input.
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{
    CandidateSelectionV1, CapabilityRequirementsV1, ComplexityClassifierModeV1, RoutingLimitsV1,
};
use crate::delegation::WorkerHarnessV1;
use crate::{AgentPlanDisplayName, AgentPlanPurpose};

pub const PLAN_EDITOR_SCHEMA_V2: &str = "hiroute.plan-editor/v2";
pub const PLAN_AUTHORING_SCHEMA_V2: &str = "hiroute.plan-authoring/v2";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanEditorMode {
    FixedModel,
    SmartSaving,
    FreeFirst,
}

/// Installation availability belongs to 14 and does not alter saved configuration.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerPlanV1 {
    pub harness: WorkerHarnessV1,
    pub protocol: crate::AgentIngressProtocolV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SmartEditorV2 {
    pub economy: Vec<CandidateSelectionV1>,
    pub primary: Vec<CandidateSelectionV1>,
    pub primary_fallback: bool,
    pub reselect_on_user_message: bool,
    pub classifier: ComplexityClassifierModeV1,
    pub complex_keywords: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FreeEditorV2 {
    pub candidates: Vec<CandidateSelectionV1>,
    pub primary: Vec<CandidateSelectionV1>,
    pub primary_fallback: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanEditorStateV2 {
    pub schema: String,
    pub display_name: String,
    pub purpose: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_alias: Option<String>,
    pub mode: PlanEditorMode,
    pub candidates: Vec<CandidateSelectionV1>,
    pub smart: SmartEditorV2,
    pub free: FreeEditorV2,
    pub delegation_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work: Option<WorkerPlanV1>,
    pub requirements: CapabilityRequirementsV1,
    pub limits: RoutingLimitsV1,
}

impl PlanEditorStateV2 {
    /// An unfinished row is legal. Strict candidate/native/protocol validation runs only on
    /// the effective configuration at Preview; all draft modes remain bounded and non-secret.
    pub fn validate_draft(&self) -> Result<(), PlanAuthoringError> {
        if self.schema != PLAN_EDITOR_SCHEMA_V2
            || self.display_name.chars().count() > 128
            || self.purpose.chars().count() > 512
            || self.custom_alias.as_ref().is_some_and(|v| v.len() > 64)
            || self.smart.complex_keywords.len() > 64
            || self.smart.classifier.validate().is_err()
            || self
                .smart
                .complex_keywords
                .iter()
                .any(|v| v.chars().count() > 64)
        {
            return Err(PlanAuthoringError::InvalidDraft);
        }
        // Keep the same metadata secret screening as published identities, while permitting
        // incomplete (including empty) non-sensitive input to be corrected by the user.
        for (text, valid) in [
            (
                &self.display_name,
                self.display_name.is_empty()
                    || AgentPlanDisplayName::parse(self.display_name.trim()).is_ok(),
            ),
            (
                &self.purpose,
                self.purpose.is_empty() || AgentPlanPurpose::parse(self.purpose.trim()).is_ok(),
            ),
        ] {
            if !valid || text.chars().any(char::is_control) {
                return Err(PlanAuthoringError::InvalidDraft);
            }
        }
        for candidates in [
            &self.candidates,
            &self.smart.economy,
            &self.smart.primary,
            &self.free.candidates,
            &self.free.primary,
        ] {
            if candidates.len() > 128
                || candidates
                    .iter()
                    .any(|c| c.binding_id.len() > 256 || c.binding_id.chars().any(char::is_control))
            {
                return Err(PlanAuthoringError::InvalidDraft);
            }
        }
        let bytes = serde_json::to_vec(self).map_err(|_| PlanAuthoringError::InvalidDraft)?;
        if bytes.len() > 262_144 {
            return Err(PlanAuthoringError::InvalidDraft);
        }
        Ok(())
    }

    pub fn effective(&self) -> Result<AgentPlanAuthoringV2, PlanAuthoringError> {
        self.validate_draft()?;
        let strategy = match self.mode {
            PlanEditorMode::FixedModel => AgentPlanStrategyV2::Custom {
                candidates: self.candidates.clone(),
            },
            PlanEditorMode::SmartSaving => AgentPlanStrategyV2::SmartSaving {
                economy: self.smart.economy.clone(),
                primary: self.smart.primary.clone(),
                primary_fallback: self.smart.primary_fallback,
                reselect_on_user_message: self.smart.reselect_on_user_message,
                classifier: self.smart.classifier.clone(),
                complex_keywords: self.smart.complex_keywords.clone(),
            },
            PlanEditorMode::FreeFirst => AgentPlanStrategyV2::FreeFirst {
                candidates: self.free.candidates.clone(),
                primary_fallback: self.free.primary_fallback,
                primary: if self.free.primary_fallback {
                    self.free.primary.clone()
                } else {
                    Vec::new()
                },
            },
        };
        let value = AgentPlanAuthoringV2 {
            schema: PLAN_AUTHORING_SCHEMA_V2.into(),
            mode: self.mode,
            display_name: AgentPlanDisplayName::parse(&self.display_name)
                .map_err(|_| PlanAuthoringError::InvalidMetadata)?,
            purpose: AgentPlanPurpose::parse(&self.purpose)
                .map_err(|_| PlanAuthoringError::InvalidMetadata)?,
            requirements: self.requirements.clone(),
            limits: self.limits.clone(),
            strategy,
            delegation_enabled: self.delegation_enabled,
            work: self.work.clone(),
        };
        value.validate()?;
        Ok(value)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPlanAuthoringV2 {
    pub schema: String,
    pub display_name: AgentPlanDisplayName,
    pub purpose: AgentPlanPurpose,
    pub mode: PlanEditorMode,
    pub requirements: CapabilityRequirementsV1,
    pub limits: RoutingLimitsV1,
    pub strategy: AgentPlanStrategyV2,
    pub delegation_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work: Option<WorkerPlanV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentPlanStrategyV2 {
    SmartSaving {
        economy: Vec<CandidateSelectionV1>,
        primary: Vec<CandidateSelectionV1>,
        primary_fallback: bool,
        reselect_on_user_message: bool,
        classifier: ComplexityClassifierModeV1,
        complex_keywords: Vec<String>,
    },
    FreeFirst {
        candidates: Vec<CandidateSelectionV1>,
        primary_fallback: bool,
        primary: Vec<CandidateSelectionV1>,
    },
    Custom {
        candidates: Vec<CandidateSelectionV1>,
    },
}

impl AgentPlanAuthoringV2 {
    pub fn validate(&self) -> Result<(), PlanAuthoringError> {
        if self.schema != PLAN_AUTHORING_SCHEMA_V2 {
            return Err(PlanAuthoringError::UnsupportedSchema);
        }
        self.display_name
            .validate()
            .map_err(|_| PlanAuthoringError::InvalidMetadata)?;
        self.purpose
            .validate()
            .map_err(|_| PlanAuthoringError::InvalidMetadata)?;
        self.requirements
            .validate()
            .map_err(|_| PlanAuthoringError::InvalidStrategy)?;
        self.limits
            .validate()
            .map_err(|_| PlanAuthoringError::InvalidStrategy)?;
        if self.delegation_enabled && self.work.is_none() {
            return Err(PlanAuthoringError::InvalidStrategy);
        }
        let validate = |items: &[CandidateSelectionV1]| {
            super::strategy::validate_candidates(items)
                .map_err(|_| PlanAuthoringError::InvalidStrategy)
        };
        match (&self.mode, &self.strategy) {
            (
                PlanEditorMode::SmartSaving,
                AgentPlanStrategyV2::SmartSaving {
                    economy,
                    primary,
                    classifier,
                    complex_keywords,
                    ..
                },
            ) => {
                validate(economy)?;
                validate(primary)?;
                classifier
                    .validate()
                    .map_err(|_| PlanAuthoringError::InvalidStrategy)?;
                super::strategy::validate_keywords(complex_keywords)
                    .map_err(|_| PlanAuthoringError::InvalidStrategy)?;
            }
            (
                PlanEditorMode::FreeFirst,
                AgentPlanStrategyV2::FreeFirst {
                    candidates,
                    primary_fallback,
                    primary,
                },
            ) => {
                validate(candidates)?;
                if *primary_fallback {
                    validate(primary)?;
                } else if !primary.is_empty() {
                    return Err(PlanAuthoringError::InvalidStrategy);
                }
            }
            (PlanEditorMode::FixedModel, AgentPlanStrategyV2::Custom { candidates }) => {
                validate(candidates)?;
            }
            _ => return Err(PlanAuthoringError::InvalidStrategy),
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum PlanAuthoringError {
    #[error("plan authoring schema is unsupported")]
    UnsupportedSchema,
    #[error("plan draft exceeds bounds or contains unsafe metadata")]
    InvalidDraft,
    #[error("plan name or purpose is invalid")]
    InvalidMetadata,
    #[error("the selected plan mode has incomplete or invalid groups")]
    InvalidStrategy,
}

#[cfg(test)]
mod tests;

mod editor;
