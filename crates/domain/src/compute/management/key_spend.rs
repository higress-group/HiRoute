//! Budget accounting uses the same immutable source price and protocol usage semantics as receipts.
use crate::{
    ExecutionPricingEvidenceV1, InputUsageMeaningV1, OutputUsageMeaningV1, PortError, TokenRateV1,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KeySpendEstimateV1 {
    pub pricing: ExecutionPricingEvidenceV1,
    pub input_bound: u64,
    pub output_bound: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KeySpendReservationV1 {
    pub reservation_id: String,
    pub budget_id: String,
    pub amount_micros: u64,
    pub budget_epoch: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KeySpendUsageV1 {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum KeySpendSettlementV1 {
    Unsent,
    Usage { usage: KeySpendUsageV1 },
    Unknown,
}

#[derive(Debug, thiserror::Error)]
pub enum KeyBudgetGateErrorV1 {
    #[error("key spending allowance exhausted")]
    Exhausted,
    #[error("a complete trusted price and bounded request are required")]
    PriceUnavailable,
    #[error("key allowance and price currencies differ")]
    CurrencyMismatch,
    #[error("key spending authority unavailable")]
    Authority(#[from] PortError),
}

impl KeySpendEstimateV1 {
    fn rates(&self) -> Option<[u64; 4]> {
        self.pricing.validate().ok()?;
        let quote = self.pricing.quote.as_ref()?;
        if !quote.unknown_reasons.is_empty() {
            return None;
        }
        fn known(rate: &TokenRateV1) -> Option<u64> {
            match rate {
                TokenRateV1::Known {
                    micros_per_million_tokens,
                } => Some(*micros_per_million_tokens),
                _ => None,
            }
        }
        Some([
            known(&quote.rates.input_uncached)?,
            known(&quote.rates.output)?,
            known(&quote.rates.cache_read)?,
            known(&quote.rates.cache_write)?,
        ])
    }
    pub fn upper_bound_micros(&self) -> Option<u64> {
        let rates = self.rates()?;
        if self.input_bound == 0 || self.output_bound == 0 {
            return None;
        }
        // A cache tier is never silently priced as zero. All input capacity is
        // reserved at the most expensive complete rate, including cache writes.
        let input = rates[0].max(rates[2]).max(rates[3]);
        micros(&[(self.input_bound, input), (self.output_bound, rates[1])])
    }
    pub fn usage_micros(&self, usage: &KeySpendUsageV1) -> Option<u64> {
        let rates = self.rates()?;
        let input = usage.input_tokens?;
        let mut output = usage.output_tokens?;
        match self.pricing.usage_semantics.output {
            OutputUsageMeaningV1::IncludesReasoning => {}
            OutputUsageMeaningV1::ReasoningSeparatelyAtOutputRate => {
                output = output.checked_add(usage.reasoning_tokens?)?
            }
            OutputUsageMeaningV1::Unknown => return None,
        }
        let mut parts = vec![(output, rates[1])];
        match self.pricing.usage_semantics.input {
            InputUsageMeaningV1::IncludesExclusiveCache => {
                if let (Some(read), Some(write)) =
                    (usage.cache_read_tokens, usage.cache_write_tokens)
                {
                    let uncached = input.checked_sub(read)?.checked_sub(write)?;
                    parts.extend([(uncached, rates[0]), (read, rates[2]), (write, rates[3])]);
                } else {
                    // Known total input still provides a conservative charge
                    // when its cache split is absent; it is not a free cache bucket.
                    parts.push((input, rates[0].max(rates[2]).max(rates[3])));
                }
            }
            InputUsageMeaningV1::UncachedOnly => parts.extend([
                (input, rates[0]),
                (usage.cache_read_tokens?, rates[2]),
                (usage.cache_write_tokens?, rates[3]),
            ]),
            InputUsageMeaningV1::Unknown => return None,
        }
        micros(&parts)
    }
}

fn micros(parts: &[(u64, u64)]) -> Option<u64> {
    let value = parts.iter().try_fold(0u128, |sum, (tokens, rate)| {
        sum.checked_add(u128::from(*tokens).checked_mul(u128::from(*rate))?)
    })?;
    u64::try_from(value.checked_add(999_999)? / 1_000_000).ok()
}

/// Synchronous durable accounting, independent of optional request telemetry.
pub trait KeySpendAuthorityV1: Send + Sync {
    fn reserve(
        &self,
        key_id: &str,
        generation: u64,
        estimate: Option<&KeySpendEstimateV1>,
    ) -> Result<Option<KeySpendReservationV1>, KeyBudgetGateErrorV1>;
    fn mark_sent(&self, id: &str) -> crate::PortResult<()>;
    fn settle(&self, id: &str, settlement: &KeySpendSettlementV1) -> crate::PortResult<()>;
}
