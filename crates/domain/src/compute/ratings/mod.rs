//! Immutable reference ratings. Callers establish trusted model identity before lookup.
//! A score is never execution capability or routing authority.
use super::common::validate_identifier;
use super::{ComputeContractError, ModelNativeReasoningV1};
use crate::{
    CanonicalDigest, ExactNativeReasoningV1, NativeReasoningCapabilityV1, ReasoningRenderModeV1,
};
use serde::{Deserialize, Serialize};
mod reference;
pub use reference::*;
mod release;
pub use release::*;
use std::collections::BTreeSet;

pub const RATING_SNAPSHOT_SCHEMA_V2: &str = "hiroute.rating-snapshot/v2";

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NativeRatingConfigurationV1 {
    Fixed { profile: String },
    Toggle { enabled: bool },
    Profile { profile: String },
    Budget { tokens: u32 },
}
impl NativeRatingConfigurationV1 {
    pub fn from_exact(value: &ExactNativeReasoningV1) -> Result<Self, ComputeContractError> {
        value
            .validate()
            .map_err(|_| ComputeContractError::InvalidModelData)?;
        Ok(match value {
            ExactNativeReasoningV1::Fixed { profile, .. } => Self::Fixed {
                profile: profile.clone(),
            },
            ExactNativeReasoningV1::Toggle { enabled, .. } => Self::Toggle { enabled: *enabled },
            ExactNativeReasoningV1::Profile { profile, .. } => Self::Profile {
                profile: profile.clone(),
            },
            ExactNativeReasoningV1::Budget { tokens, .. } => Self::Budget { tokens: *tokens },
        })
    }
    fn supported_by(&self, capability: &NativeReasoningCapabilityV1) -> bool {
        match (self, capability) {
            (Self::Fixed { profile }, NativeReasoningCapabilityV1::Fixed { profile: p }) => {
                profile == p
            }
            (Self::Toggle { .. }, NativeReasoningCapabilityV1::Toggle { .. }) => true,
            (Self::Profile { profile }, NativeReasoningCapabilityV1::Discrete { profiles, .. }) => {
                profiles.contains(profile)
            }
            (
                Self::Budget { tokens },
                NativeReasoningCapabilityV1::Budget {
                    minimum_tokens,
                    maximum_tokens,
                    step_tokens,
                    ..
                },
            ) => {
                *step_tokens > 0
                    && tokens >= minimum_tokens
                    && tokens <= maximum_tokens
                    && (tokens - minimum_tokens) % step_tokens == 0
            }
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RatingUnknownReasonV1 {
    IdentityUnresolved,
    ConfigurationNotInSnapshot,
    RatingNotCollected,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum RatingValueV1 {
    Reference {
        score_tenths: u8,
        evidence_ref: String,
        method_revision: String,
    },
    Estimated {
        score_tenths: u8,
        evidence_ref: String,
        method_revision: String,
    },
    Unknown {
        reason: RatingUnknownReasonV1,
    },
}
impl RatingValueV1 {
    pub fn unknown(reason: RatingUnknownReasonV1) -> Self {
        Self::Unknown { reason }
    }
    fn validate(&self) -> Result<(), ComputeContractError> {
        match self {
            Self::Reference {
                score_tenths,
                evidence_ref,
                method_revision,
            }
            | Self::Estimated {
                score_tenths,
                evidence_ref,
                method_revision,
            } => {
                if !(5..=50).contains(score_tenths) {
                    return Err(ComputeContractError::InvalidModelData);
                }
                validate_identifier(evidence_ref)?;
                validate_identifier(method_revision)
            }
            Self::Unknown { .. } => Ok(()),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeConfigurationRatingV2 {
    pub model_configuration_id: String,
    pub native_configuration: NativeRatingConfigurationV1,
    pub overall: RatingValueV1,
    pub coding: RatingValueV1,
    pub tool: RatingValueV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RatingSnapshotV2 {
    pub schema: String,
    pub version: String,
    pub scale_version: String,
    pub model_catalog_digest: CanonicalDigest,
    pub models: Vec<ModelNativeReasoningV1>,
    pub records: Vec<NativeConfigurationRatingV2>,
    pub digest: CanonicalDigest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RatingSnapshotRefV1 {
    pub version: String,
    pub digest: CanonicalDigest,
    pub scale_version: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedModelRatingV1 {
    pub requested_model: String,
    pub requested_configuration: NativeRatingConfigurationV1,
    pub matched_model: Option<String>,
    pub matched_configuration: Option<NativeRatingConfigurationV1>,
    pub overall: RatingValueV1,
    pub coding: RatingValueV1,
    pub tool: RatingValueV1,
}

impl RatingSnapshotV2 {
    pub fn computed_digest(&self) -> Result<CanonicalDigest, ComputeContractError> {
        CanonicalDigest::of(&(
            &self.schema,
            &self.version,
            &self.scale_version,
            &self.model_catalog_digest,
            &self.models,
            &self.records,
        ))
        .map_err(|_| ComputeContractError::InvalidModelData)
    }
    pub fn snapshot_ref(&self) -> RatingSnapshotRefV1 {
        RatingSnapshotRefV1 {
            version: self.version.clone(),
            digest: self.digest.clone(),
            scale_version: self.scale_version.clone(),
        }
    }
    pub fn validate(&self) -> Result<(), ComputeContractError> {
        if self.schema != RATING_SNAPSHOT_SCHEMA_V2 {
            return Err(ComputeContractError::UnsupportedSchema);
        }
        if serde_json::to_vec(self)
            .map_err(|_| ComputeContractError::InvalidModelData)?
            .len()
            > super::MAX_RELEASE_BUNDLE_BYTES
        {
            return Err(ComputeContractError::InvalidModelData);
        }
        validate_identifier(&self.version)?;
        validate_identifier(&self.scale_version)?;
        super::common::validate_evidence(self.model_catalog_digest.as_str())?;
        if self.digest != self.computed_digest()? {
            return Err(ComputeContractError::InvalidEvidence);
        }
        let mut models = std::collections::BTreeMap::new();
        for model in &self.models {
            validate_identifier(&model.model_configuration_id)?;
            model
                .validate()
                .map_err(|_| ComputeContractError::InvalidModelData)?;
            if models
                .insert(&model.model_configuration_id, &model.capability)
                .is_some()
            {
                return Err(ComputeContractError::DuplicateIdentity);
            }
        }
        let mut keys = BTreeSet::new();
        for record in &self.records {
            let capability = models
                .get(&record.model_configuration_id)
                .ok_or(ComputeContractError::CrossReference)?;
            if !record.native_configuration.supported_by(capability) {
                return Err(ComputeContractError::InvalidModelData);
            }
            if !keys.insert((&record.model_configuration_id, &record.native_configuration)) {
                return Err(ComputeContractError::DuplicateIdentity);
            }
            for value in [&record.overall, &record.coding, &record.tool] {
                value.validate()?;
            }
        }
        Ok(())
    }
    /// Pure lookup on an already validated snapshot. It does not validate current capability:
    /// a valid execution configuration may have been introduced after this rating snapshot.
    pub fn resolve(
        &self,
        model: &str,
        exact: &ExactNativeReasoningV1,
    ) -> Result<ResolvedModelRatingV1, ComputeContractError> {
        validate_identifier(model)?;
        let config = NativeRatingConfigurationV1::from_exact(exact)?;
        let record = self
            .records
            .iter()
            .find(|r| r.model_configuration_id == model && r.native_configuration == config);
        let known_model = self
            .models
            .iter()
            .any(|m| m.model_configuration_id == model);
        let unknown = RatingValueV1::unknown(if known_model {
            RatingUnknownReasonV1::ConfigurationNotInSnapshot
        } else {
            RatingUnknownReasonV1::IdentityUnresolved
        });
        Ok(ResolvedModelRatingV1 {
            requested_model: model.into(),
            requested_configuration: config,
            matched_model: record.map(|r| r.model_configuration_id.clone()),
            matched_configuration: record.map(|r| r.native_configuration.clone()),
            overall: record.map_or_else(|| unknown.clone(), |r| r.overall.clone()),
            coding: record.map_or_else(|| unknown.clone(), |r| r.coding.clone()),
            tool: record.map_or(unknown, |r| r.tool.clone()),
        })
    }
}

/// V1 evidence is projected only when its native effort is explicit. No default expansion.
pub fn legacy_rating_configuration(
    capability: &NativeReasoningCapabilityV1,
    effort: &str,
) -> Option<NativeRatingConfigurationV1> {
    if matches!(effort, "default" | "provider-default") || capability.validate().is_err() {
        return None;
    }
    let value = match capability {
        NativeReasoningCapabilityV1::Fixed { profile } if profile == effort => {
            ExactNativeReasoningV1::Fixed {
                profile: profile.clone(),
                render_mode: ReasoningRenderModeV1::NoControlParameter,
            }
        }
        NativeReasoningCapabilityV1::Toggle { parameter }
            if matches!(effort, "enabled" | "disabled") =>
        {
            ExactNativeReasoningV1::Toggle {
                parameter: parameter.clone(),
                enabled: effort == "enabled",
                render_mode: ReasoningRenderModeV1::ExplicitNative,
            }
        }
        NativeReasoningCapabilityV1::Discrete {
            parameter,
            profiles,
            ..
        } if profiles.iter().any(|p| p == effort) => ExactNativeReasoningV1::Profile {
            parameter: parameter.clone(),
            profile: effort.into(),
            render_mode: ReasoningRenderModeV1::ExplicitNative,
        },
        NativeReasoningCapabilityV1::Budget { parameter, .. } => ExactNativeReasoningV1::Budget {
            parameter: parameter.clone(),
            tokens: effort.parse().ok()?,
            render_mode: ReasoningRenderModeV1::ExplicitNative,
        },
        _ => return None,
    };
    let config = NativeRatingConfigurationV1::from_exact(&value).ok()?;
    config.supported_by(capability).then_some(config)
}

#[cfg(test)]
mod tests;
