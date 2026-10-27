//! Deterministic valuation of retained execution evidence. This module never
//! captures current prices or modifies an immutable Gateway receipt.

mod arithmetic;
mod comparison;
mod reference_usage;
pub use comparison::{ReferenceComparisonV2, compare_reference};
pub(crate) use reference_usage::{
    reference_bounds_semantics, reference_usage, reference_usage_semantics,
};
mod ingest;
mod retention;
mod schema;
mod usage_archive;
mod worker;
pub(crate) use ingest::{RawUsage, ingest};
pub(crate) use retention::{archive_request, archive_session};
pub(crate) use schema::migrate;
pub use worker::{ValuationAmountV2, ValuationRecordV2};
#[cfg(test)]
mod tests;

use hiroute_domain::{FrozenPriceQuoteV1, PriceValuationKindV1};
use serde::{Deserialize, Serialize};

pub use arithmetic::value_attempt;
pub(crate) use arithmetic::{amount_only, validate_quote, value_recorded_attempt};
pub const VALUATION_ALGORITHM: &str = "hiroute.valuation/2-micros-half-up-per-attempt";
pub(crate) const EQUAL_RATE_VALUATION_ALGORITHM: &str =
    "hiroute.valuation/3-equal-rate-micros-half-up-per-attempt";

pub(crate) const CACHE_HIT_ARCHIVE_SCHEMA: &str = "cache_hit_schema_v1";
pub(crate) const CACHE_HIT_TOTAL_ATTEMPTS: &str = "cache_hit_total_attempts";
pub(crate) const CACHE_HIT_ELIGIBLE_ATTEMPTS: &str = "cache_hit_eligible_attempts";
pub(crate) const CACHE_HIT_ZERO_INPUT_ATTEMPTS: &str = "cache_hit_zero_input_attempts";
pub(crate) const CACHE_HIT_MISSING_ATTEMPTS: &str = "cache_hit_missing_attempts";
pub(crate) const CACHE_HIT_INVALID_ATTEMPTS: &str = "cache_hit_invalid_attempts";
pub(crate) const CACHE_HIT_READ_TOKENS: &str = "cache_hit_read_tokens";
pub(crate) const CACHE_HIT_INPUT_TOKENS: &str = "cache_hit_input_tokens";

/// Mutually exclusive billing buckets, proven by the protocol adapter. An
/// unknown inclusion relationship must remain None, even if total input exists.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageBucketsV2 {
    pub input_uncached: Option<u64>,
    pub output: Option<u64>,
    pub cache_read: Option<u64>,
    pub cache_write: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricCoverage {
    Complete,
    Partial,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValuationMissingReason {
    UsageUnknown,
    PriceUnknown,
    ArithmeticOverflow,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ValuationComponentV2 {
    pub bucket: String,
    pub tokens: Option<u64>,
    pub missing: Option<ValuationMissingReason>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AttemptValuationV2 {
    pub algorithm: String,
    pub currency: String,
    pub valuation_kind: PriceValuationKindV1,
    pub quote: FrozenPriceQuoteV1,
    pub known_sum_micros: Option<u64>,
    /// Coverage of the amount. Individual token components can remain unknown
    /// when equal frozen rates make the combined amount determinable.
    pub coverage: MetricCoverage,
    pub components: Vec<ValuationComponentV2>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ValuationError {
    #[error("valuation input does not match validated frozen evidence")]
    InvalidEvidence,
    #[error("valuation input is out of bounds")]
    InvalidInput,
    #[error("valuation storage is unavailable")]
    Storage,
}

impl From<rusqlite::Error> for ValuationError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Storage
    }
}

impl UsageBucketsV2 {
    /// Only call for a protocol that confirms both inclusion and mutually
    /// exclusive cache counts. Unknown cache counts cannot become uncached input.
    pub fn from_inclusive_input(
        total_input: Option<u64>,
        output: Option<u64>,
        cache_read: Option<u64>,
        cache_write: Option<u64>,
    ) -> Self {
        let input_uncached = total_input
            .zip(cache_read)
            .zip(cache_write)
            .and_then(|((total, read), write)| total.checked_sub(read)?.checked_sub(write));
        // Inconsistent known counts invalidate all input buckets, preventing a
        // misleading subtotal from an impossible adapter measurement.
        let inconsistent = total_input.is_some()
            && cache_read.is_some()
            && cache_write.is_some()
            && input_uncached.is_none();
        Self {
            input_uncached,
            output,
            cache_read: if inconsistent { None } else { cache_read },
            cache_write: if inconsistent { None } else { cache_write },
        }
    }
}
