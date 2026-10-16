//! Source-scoped prices and portable frozen evidence. These values grant no execution rights.
use super::common::{validate_evidence, validate_identifier};
use super::{ComputeContractError, PriceScheduleV1};
use crate::{CanonicalDigest, WorkspaceId};
use serde::{Deserialize, Serialize};
mod change;
pub use change::*;

#[derive(
    schemars::JsonSchema, Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(
    tag = "kind",
    content = "id",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum PriceModelIdentityV1 {
    CatalogModel(String),
    LocalModel(String),
}
#[derive(
    schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum PriceValuationKindV1 {
    UsageEstimate,
    ApiEquivalent,
}
#[derive(
    schemars::JsonSchema, Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(deny_unknown_fields)]
pub struct PriceTargetV1 {
    pub workspace_id: WorkspaceId,
    pub source_id: String,
    pub source_identity_digest: CanonicalDigest,
    pub model_identity: PriceModelIdentityV1,
    pub currency: String,
    pub valuation_kind: PriceValuationKindV1,
}
impl PriceTargetV1 {
    pub fn validate(&self) -> Result<(), ComputeContractError> {
        WorkspaceId::parse(self.workspace_id.as_str())
            .map_err(|_| ComputeContractError::InvalidPrice)?;
        validate_identifier(&self.source_id)?;
        validate_evidence(self.source_identity_digest.as_str())?;
        match &self.model_identity {
            PriceModelIdentityV1::CatalogModel(id) | PriceModelIdentityV1::LocalModel(id) => {
                validate_identifier(id)?
            }
        }
        if self.currency.len() != 3 || !self.currency.bytes().all(|b| b.is_ascii_uppercase()) {
            return Err(ComputeContractError::InvalidPrice);
        }
        Ok(())
    }
    pub fn digest(&self) -> Result<CanonicalDigest, ComputeContractError> {
        self.validate()?;
        CanonicalDigest::of(self).map_err(|_| ComputeContractError::InvalidPrice)
    }
}
#[derive(
    schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum PriceUnknownReasonV1 {
    SnapshotUnavailable,
    TargetNotInSnapshot,
    PriceNotCollected,
    CacheRateNotCollected,
    UnsupportedBillingCondition,
    InvalidExecutionTime,
    AmbiguousRule,
    DisabledRule,
    UnverifiedOfferMapping,
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum TokenRateV1 {
    Known { micros_per_million_tokens: u64 },
    Unknown { reason: PriceUnknownReasonV1 },
}
impl TokenRateV1 {
    pub fn known(value: u64) -> Self {
        Self::Known {
            micros_per_million_tokens: value,
        }
    }
    pub fn unknown(reason: PriceUnknownReasonV1) -> Self {
        Self::Unknown { reason }
    }
    /// Decimal currency units per million tokens. No float or rounding is involved.
    pub fn parse_decimal(value: &str) -> Result<Self, ComputeContractError> {
        if value.is_empty() || value.len() > 27 {
            return Err(ComputeContractError::InvalidPrice);
        }
        let mut parts = value.split('.');
        let whole = parts.next().unwrap_or_default();
        let fraction = parts.next();
        if parts.next().is_some()
            || whole.is_empty()
            || !whole.bytes().all(|b| b.is_ascii_digit())
            || fraction.is_some_and(|f| {
                f.is_empty() || f.len() > 6 || !f.bytes().all(|b| b.is_ascii_digit())
            })
        {
            return Err(ComputeContractError::InvalidPrice);
        }
        let whole: u64 = whole
            .parse()
            .map_err(|_| ComputeContractError::InvalidPrice)?;
        let fraction_value = match fraction {
            Some(f) => {
                f.parse::<u64>()
                    .map_err(|_| ComputeContractError::InvalidPrice)?
                    * 10u64.pow(6 - f.len() as u32)
            }
            None => 0,
        };
        let amount = whole
            .checked_mul(1_000_000)
            .and_then(|w| w.checked_add(fraction_value))
            .ok_or(ComputeContractError::InvalidPrice)?;
        Ok(Self::known(amount))
    }
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TokenRatesV1 {
    pub input_uncached: TokenRateV1,
    pub output: TokenRateV1,
    pub cache_read: TokenRateV1,
    pub cache_write: TokenRateV1,
}
impl TokenRatesV1 {
    pub fn unknown(reason: PriceUnknownReasonV1) -> Self {
        Self {
            input_uncached: TokenRateV1::unknown(reason),
            output: TokenRateV1::unknown(reason),
            cache_read: TokenRateV1::unknown(reason),
            cache_write: TokenRateV1::unknown(reason),
        }
    }
    pub fn from_legacy(input: u64, output: u64) -> Self {
        Self {
            input_uncached: TokenRateV1::known(input),
            output: TokenRateV1::known(output),
            cache_read: TokenRateV1::unknown(PriceUnknownReasonV1::CacheRateNotCollected),
            cache_write: TokenRateV1::unknown(PriceUnknownReasonV1::CacheRateNotCollected),
        }
    }
    pub fn validate_manual(&self) -> Result<(), ComputeContractError> {
        if matches!(self.input_uncached, TokenRateV1::Known { .. })
            && matches!(self.output, TokenRateV1::Known { .. })
        {
            Ok(())
        } else {
            Err(ComputeContractError::InvalidPrice)
        }
    }
    pub fn unknown_reasons(&self) -> Vec<PriceUnknownReasonV1> {
        [
            &self.input_uncached,
            &self.output,
            &self.cache_read,
            &self.cache_write,
        ]
        .iter()
        .filter_map(|r| match r {
            TokenRateV1::Unknown { reason } => Some(*reason),
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
    }
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SourcePriceSettingV1 {
    Set { rates: TokenRatesV1 },
    FollowCatalog,
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourcePriceOverrideV1 {
    pub target: PriceTargetV1,
    pub revision: u64,
    pub setting: SourcePriceSettingV1,
}
impl SourcePriceOverrideV1 {
    pub fn validate(&self) -> Result<(), ComputeContractError> {
        self.target.validate()?;
        if self.revision == 0 {
            return Err(ComputeContractError::InvalidPrice);
        }
        if let SourcePriceSettingV1::Set { rates } = &self.setting {
            rates.validate_manual()?;
        }
        Ok(())
    }
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PriceFactRefV1 {
    pub id: String,
    pub revision: u64,
    pub digest: CanonicalDigest,
}
impl PriceFactRefV1 {
    pub fn validate(&self) -> Result<(), ComputeContractError> {
        validate_identifier(&self.id)?;
        validate_evidence(self.digest.as_str())?;
        if self.revision == 0 {
            return Err(ComputeContractError::InvalidPrice);
        }
        Ok(())
    }
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PriceGenerationRefV1 {
    pub id: String,
    pub digest: CanonicalDigest,
    pub configuration_revision: u64,
    pub catalog_refs: Vec<PriceFactRefV1>,
}
#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PriceOriginV1 {
    Manual,
    Catalog,
    LegacyOverride,
    ApiReference,
    Unknown,
}
#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PriceUnitV1 {
    MicrosPerMillionTokens,
}
/// Verified adapter facts must supply this context. An unsupported condition keeps price unknown.
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PriceBillingContextV1 {
    StandardTokens,
    Unsupported { condition: String },
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenPriceQuoteV1 {
    pub generation_ref: Option<PriceGenerationRefV1>,
    pub exact_target: PriceTargetV1,
    pub actual_source_ref: String,
    pub actual_offer_ref: Option<String>,
    pub reference_model_offer_ref: Option<PriceReferenceOfferV1>,
    pub valuation_kind: PriceValuationKindV1,
    pub currency: String,
    pub unit: PriceUnitV1,
    pub rates: TokenRatesV1,
    pub origin: PriceOriginV1,
    pub applied_override_refs: Vec<PriceFactRefV1>,
    pub selected_rule_refs: Vec<PriceFactRefV1>,
    pub attempt_execution_at: i64,
    pub applied_schedule: Option<PriceScheduleV1>,
    pub billing_context: PriceBillingContextV1,
    pub unknown_reasons: Vec<PriceUnknownReasonV1>,
    pub quote_digest: CanonicalDigest,
}
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PriceReferenceOfferV1 {
    pub model_configuration_id: String,
    pub offer_ref: String,
}
impl FrozenPriceQuoteV1 {
    pub fn computed_digest(&self) -> Result<CanonicalDigest, ComputeContractError> {
        let mut value =
            serde_json::to_value(self).map_err(|_| ComputeContractError::InvalidPrice)?;
        value
            .as_object_mut()
            .ok_or(ComputeContractError::InvalidPrice)?
            .remove("quote_digest");
        CanonicalDigest::of(&value).map_err(|_| ComputeContractError::InvalidPrice)
    }
    pub fn verify_digest(&self) -> Result<(), ComputeContractError> {
        if self.computed_digest()? == self.quote_digest {
            Ok(())
        } else {
            Err(ComputeContractError::InvalidEvidence)
        }
    }
}
#[cfg(test)]
mod tests;
