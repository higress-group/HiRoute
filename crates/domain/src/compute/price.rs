use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::CanonicalDigest;

use super::common::*;
use super::model_data::ModelDataBundleV1;

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PriceScheduleV1 {
    pub timezone_id: String,
    pub timezone_data_revision: String,
    pub schedule_compile_version: String,
    pub effective_from: i64,
    pub effective_until: i64,
    pub utc_offset_minutes: i16,
    pub weekdays: BTreeSet<u8>,
    pub start_minute: u16,
    pub end_minute: u16,
}

impl PriceScheduleV1 {
    pub fn matches(&self, unix_seconds: i64) -> bool {
        if !(self.effective_from..self.effective_until).contains(&unix_seconds) {
            return false;
        }
        let Some(local_seconds) = unix_seconds.checked_add(i64::from(self.utc_offset_minutes) * 60)
        else {
            return false;
        };
        let days = local_seconds.div_euclid(86_400);
        let weekday = (days + 3).rem_euclid(7) as u8 + 1;
        let minute = (local_seconds.rem_euclid(86_400) / 60) as u16;
        if self.start_minute < self.end_minute {
            self.weekdays.contains(&weekday)
                && minute >= self.start_minute
                && minute < self.end_minute
        } else {
            (minute >= self.start_minute && self.weekdays.contains(&weekday))
                || (minute < self.end_minute
                    && self
                        .weekdays
                        .contains(&if weekday == 1 { 7 } else { weekday - 1 }))
        }
    }
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PriceRateV1 {
    pub price_rate_id: String,
    pub revision: u64,
    pub offer_ref: String,
    pub model_configuration_id: String,
    pub currency: String,
    pub input_micros_per_million: u64,
    pub output_micros_per_million: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<PriceScheduleV1>,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PriceOverrideOperation {
    Replace,
    MultiplyPartsPerMillion,
    DisableCatalogRule,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PriceOverrideV1 {
    pub override_id: String,
    pub revision: u64,
    pub offer_ref: String,
    pub model_configuration_id: String,
    /// Overrides are exact-currency facts; a USD override never changes a CNY price.
    pub currency: String,
    pub operation: PriceOverrideOperation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_rule_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_value: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_value: Option<u64>,
}

impl PriceOverrideV1 {
    pub fn validate(&self) -> Result<(), ComputeContractError> {
        validate_price_override(self)
    }
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AppliedPriceOverrideV1 {
    pub override_id: String,
    pub revision: u64,
    pub operation: PriceOverrideOperation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_rule_id: Option<String>,
    /// Digest of the complete revisioned override, not merely its stable ID.
    pub semantic_digest: CanonicalDigest,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EffectivePriceV1 {
    pub base_price_rate_id: String,
    pub base_price_rate_revision: u64,
    pub price_rate_id: String,
    pub price_rate_revision: u64,
    pub offer_ref: String,
    pub model_configuration_id: String,
    pub currency: String,
    pub input_micros_per_million: u64,
    pub output_micros_per_million: u64,
    pub model_data_bundle_version: String,
    pub model_data_digest: CanonicalDigest,
    pub price_slice_version: String,
    pub routing_evaluated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied_schedule: Option<PriceScheduleV1>,
    /// Exact revisions and semantic digests for every override participating in the fold.
    #[serde(default)]
    pub applied_overrides: Vec<AppliedPriceOverrideV1>,
    pub frozen_digest: CanonicalDigest,
}

pub fn effective_price(
    model_data: &ModelDataBundleV1,
    overrides: &[PriceOverrideV1],
    offer_ref: &str,
    model_configuration_id: &str,
    currency: &str,
    unix_seconds: i64,
) -> Result<EffectivePriceV1, ComputeContractError> {
    validate_identifier(offer_ref)?;
    validate_identifier(model_configuration_id)?;
    validate_currency(currency)?;
    validate_rate_sets(&model_data.price_rates)?;

    let group = model_data
        .price_rates
        .iter()
        .filter(|rate| {
            rate.offer_ref == offer_ref
                && rate.model_configuration_id == model_configuration_id
                && rate.currency == currency
        })
        .collect::<Vec<_>>();
    if group.is_empty() {
        return Err(ComputeContractError::PriceNotFound);
    }

    let mut latest = BTreeMap::<&str, &PriceOverrideV1>::new();
    for value in overrides.iter().filter(|value| {
        value.offer_ref == offer_ref
            && value.model_configuration_id == model_configuration_id
            && value.currency == currency
    }) {
        validate_price_override(value)?;
        match latest.get(value.override_id.as_str()) {
            Some(existing) if existing.revision == value.revision && *existing != value => {
                return Err(ComputeContractError::AmbiguousPrice);
            }
            Some(existing) if existing.revision >= value.revision => {}
            _ => {
                latest.insert(value.override_id.as_str(), value);
            }
        }
    }

    let mut disabled = BTreeSet::new();
    let mut value_override = None;
    for value in latest.values().copied() {
        match value.operation {
            PriceOverrideOperation::DisableCatalogRule => {
                let target = value
                    .target_rule_id
                    .as_deref()
                    .ok_or(ComputeContractError::InvalidPrice)?;
                if !group.iter().any(|rate| rate.price_rate_id == target) {
                    return Err(ComputeContractError::InvalidPrice);
                }
                disabled.insert(target);
            }
            PriceOverrideOperation::Replace | PriceOverrideOperation::MultiplyPartsPerMillion => {
                if value_override.replace(value).is_some() {
                    return Err(ComputeContractError::AmbiguousPrice);
                }
            }
        }
    }

    // Disable precedes selection so disabling a schedule falls back to the base rule.
    let enabled = group
        .iter()
        .copied()
        .filter(|rate| !disabled.contains(rate.price_rate_id.as_str()))
        .collect::<Vec<_>>();
    let bases = enabled
        .iter()
        .copied()
        .filter(|rate| rate.schedule.is_none())
        .collect::<Vec<_>>();
    let [base] = bases.as_slice() else {
        return Err(if bases.is_empty() {
            ComputeContractError::PriceDisabled
        } else {
            ComputeContractError::AmbiguousPrice
        });
    };
    let scheduled = enabled
        .iter()
        .copied()
        .filter(|rate| {
            rate.schedule
                .as_ref()
                .is_some_and(|schedule| schedule.matches(unix_seconds))
        })
        .collect::<Vec<_>>();
    let rate = match scheduled.as_slice() {
        [] => *base,
        [rate] => *rate,
        _ => return Err(ComputeContractError::AmbiguousPrice),
    };

    let mut input = rate.input_micros_per_million;
    let mut output = rate.output_micros_per_million;
    if let Some(value) = value_override {
        match value.operation {
            PriceOverrideOperation::Replace => {
                input = value
                    .input_value
                    .ok_or(ComputeContractError::InvalidPrice)?;
                output = value
                    .output_value
                    .ok_or(ComputeContractError::InvalidPrice)?;
            }
            PriceOverrideOperation::MultiplyPartsPerMillion => {
                let factor = value
                    .input_value
                    .ok_or(ComputeContractError::InvalidPrice)?;
                input = input
                    .checked_mul(factor)
                    .ok_or(ComputeContractError::InvalidPrice)?
                    / 1_000_000;
                output = output
                    .checked_mul(factor)
                    .ok_or(ComputeContractError::InvalidPrice)?
                    / 1_000_000;
            }
            PriceOverrideOperation::DisableCatalogRule => unreachable!("classified above"),
        }
    }

    let applied_overrides = latest
        .values()
        .map(|value| {
            Ok(AppliedPriceOverrideV1 {
                override_id: value.override_id.clone(),
                revision: value.revision,
                operation: value.operation,
                target_rule_id: value.target_rule_id.clone(),
                semantic_digest: CanonicalDigest::of(*value)
                    .map_err(|_| ComputeContractError::InvalidPrice)?,
            })
        })
        .collect::<Result<Vec<_>, ComputeContractError>>()?;
    let mut result = EffectivePriceV1 {
        base_price_rate_id: base.price_rate_id.clone(),
        base_price_rate_revision: base.revision,
        price_rate_id: rate.price_rate_id.clone(),
        price_rate_revision: rate.revision,
        offer_ref: offer_ref.to_owned(),
        model_configuration_id: model_configuration_id.to_owned(),
        currency: currency.to_owned(),
        input_micros_per_million: input,
        output_micros_per_million: output,
        model_data_bundle_version: model_data.bundle_version.clone(),
        model_data_digest: CanonicalDigest::of(model_data)
            .map_err(|_| ComputeContractError::InvalidPrice)?,
        price_slice_version: model_data.prices_slice_version.clone(),
        routing_evaluated_at: unix_seconds,
        applied_schedule: rate.schedule.clone(),
        applied_overrides,
        frozen_digest: CanonicalDigest::of_bytes(b"pending"),
    };
    result.frozen_digest = effective_price_digest(&result)?;
    Ok(result)
}

impl EffectivePriceV1 {
    pub fn verify_frozen_digest(&self) -> Result<(), ComputeContractError> {
        if effective_price_digest(self)? == self.frozen_digest {
            Ok(())
        } else {
            Err(ComputeContractError::InvalidPrice)
        }
    }
}

fn effective_price_digest(
    value: &EffectivePriceV1,
) -> Result<CanonicalDigest, ComputeContractError> {
    CanonicalDigest::of(&(
        &value.offer_ref,
        &value.base_price_rate_id,
        value.base_price_rate_revision,
        &value.price_rate_id,
        value.price_rate_revision,
        &value.model_configuration_id,
        &value.currency,
        value.input_micros_per_million,
        value.output_micros_per_million,
        &value.model_data_bundle_version,
        &value.model_data_digest,
        &value.price_slice_version,
        value.routing_evaluated_at,
        &value.applied_schedule,
        &value.applied_overrides,
    ))
    .map_err(|_| ComputeContractError::InvalidPrice)
}

fn validate_currency(currency: &str) -> Result<(), ComputeContractError> {
    if currency.len() == 3 && currency.bytes().all(|byte| byte.is_ascii_uppercase()) {
        Ok(())
    } else {
        Err(ComputeContractError::InvalidPrice)
    }
}

fn validate_price_override(value: &PriceOverrideV1) -> Result<(), ComputeContractError> {
    validate_identifier(&value.override_id)?;
    validate_identifier(&value.offer_ref)?;
    validate_identifier(&value.model_configuration_id)?;
    validate_currency(&value.currency)?;
    if value.revision == 0 {
        return Err(ComputeContractError::InvalidPrice);
    }
    let valid_values = match value.operation {
        PriceOverrideOperation::Replace => {
            value.target_rule_id.is_none()
                && value.input_value.is_some()
                && value.output_value.is_some()
        }
        PriceOverrideOperation::MultiplyPartsPerMillion => {
            value.target_rule_id.is_none()
                && value.input_value.is_some_and(|factor| factor > 0)
                && value.output_value.is_none()
        }
        PriceOverrideOperation::DisableCatalogRule => {
            value
                .target_rule_id
                .as_deref()
                .is_some_and(|id| validate_identifier(id).is_ok())
                && value.input_value.is_none()
                && value.output_value.is_none()
        }
    };
    if valid_values {
        Ok(())
    } else {
        Err(ComputeContractError::InvalidPrice)
    }
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PriceTracking {
    FollowLatest,
}

pub(super) fn validate_schedule(schedule: &PriceScheduleV1) -> Result<(), ComputeContractError> {
    validate_identifier(&schedule.timezone_id)?;
    validate_identifier(&schedule.timezone_data_revision)?;
    validate_identifier(&schedule.schedule_compile_version)?;
    if schedule.weekdays.is_empty()
        || schedule.weekdays.iter().any(|day| !(1..=7).contains(day))
        || schedule.start_minute > 1439
        || schedule.end_minute > 1439
        || schedule.start_minute == schedule.end_minute
        || schedule.effective_from >= schedule.effective_until
        || !(-840..=840).contains(&schedule.utc_offset_minutes)
    {
        return Err(ComputeContractError::InvalidPrice);
    }
    Ok(())
}

pub(super) fn validate_rate_sets(rates: &[PriceRateV1]) -> Result<(), ComputeContractError> {
    let mut groups = BTreeMap::<(&str, &str, &str), Vec<&PriceRateV1>>::new();
    for rate in rates {
        validate_identifier(&rate.price_rate_id)?;
        validate_identifier(&rate.offer_ref)?;
        validate_identifier(&rate.model_configuration_id)?;
        validate_currency(&rate.currency)?;
        if rate.revision == 0 {
            return Err(ComputeContractError::InvalidPrice);
        }
        if let Some(schedule) = &rate.schedule {
            validate_schedule(schedule)?;
        }
        groups
            .entry((
                rate.offer_ref.as_str(),
                rate.model_configuration_id.as_str(),
                rate.currency.as_str(),
            ))
            .or_default()
            .push(rate);
    }
    for group in groups.values() {
        if group.iter().filter(|rate| rate.schedule.is_none()).count() != 1 {
            return Err(ComputeContractError::InvalidPrice);
        }
        let scheduled = group
            .iter()
            .filter_map(|rate| rate.schedule.as_ref())
            .collect::<Vec<_>>();
        for (index, left) in scheduled.iter().enumerate() {
            for right in &scheduled[index + 1..] {
                if schedules_overlap(left, right) {
                    return Err(ComputeContractError::AmbiguousPrice);
                }
            }
        }
    }
    Ok(())
}

fn schedules_overlap(left: &PriceScheduleV1, right: &PriceScheduleV1) -> bool {
    let start = left.effective_from.max(right.effective_from);
    let end = left.effective_until.min(right.effective_until);
    if start >= end {
        return false;
    }
    let horizon = end.min(start.saturating_add(7 * 86_400 + 60));
    let mut instant = start;
    while instant < horizon {
        if left.matches(instant) && right.matches(instant) {
            return true;
        }
        let next = instant.div_euclid(60).saturating_add(1).saturating_mul(60);
        if next <= instant {
            break;
        }
        instant = next;
    }
    false
}
