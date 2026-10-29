//! Amount-only normalization for an explicitly selected current-price comparison.
//! Equal-rate buckets can be combined algebraically; stored usage is never filled in.
use super::{MetricCoverage, RawUsage, UsageBucketsV2, amount_only};
use hiroute_domain::{
    ExecutionPricingEvidenceV1, FrozenPriceQuoteV1, InputUsageMeaningV1, TokenRateV1,
    UsageFrameKindV1,
};

pub(crate) fn reference_usage(
    pricing: &ExecutionPricingEvidenceV1,
    raw: &RawUsage,
    quote: &FrozenPriceQuoteV1,
) -> UsageBucketsV2 {
    reference_usage_semantics(pricing.usage_semantics, raw, quote)
}

pub(crate) fn reference_usage_semantics(
    semantics: hiroute_domain::UsageSemanticsV1,
    raw: &RawUsage,
    quote: &FrozenPriceQuoteV1,
) -> UsageBucketsV2 {
    let mut usage = super::worker::normalize_semantics(semantics, raw);
    if semantics.frame_kind == UsageFrameKindV1::Unknown
        || semantics.input != InputUsageMeaningV1::IncludesExclusiveCache
        || !semantics.cache_buckets_exclusive
    {
        return usage;
    }
    let TokenRateV1::Known {
        micros_per_million_tokens: base,
    } = quote.rates.input_uncached
    else {
        return usage;
    };
    let equal = |rate: &TokenRateV1| matches!(rate,TokenRateV1::Known {micros_per_million_tokens} if *micros_per_million_tokens==base);
    let combine_read = raw.read.is_none() && equal(&quote.rates.cache_read);
    let combine_write = raw.write.is_none() && equal(&quote.rates.cache_write);
    if (raw.read.is_some() || combine_read)
        && (raw.write.is_some() || combine_write)
        && (combine_read || combine_write)
    {
        // With equal input/creation rates, (total-read-write)*base + write*base
        // equals (total-read)*base, regardless of the unreported creation split.
        // Zeros below denote combined calculation buckets, never measured cache counts.
        let read = if combine_read { Some(0) } else { raw.read };
        let write = if combine_write { Some(0) } else { raw.write };
        if let Some(remaining) = raw
            .input
            .zip(read)
            .zip(write)
            .and_then(|((total, r), w)| total.checked_sub(r)?.checked_sub(w))
        {
            usage.input_uncached = Some(remaining);
            usage.cache_read = read;
            usage.cache_write = write;
        }
    }
    usage
}

/// Bounds a same-usage reference when only the exclusive creation split is
/// missing. These are calculation scenarios, never reported usage facts.
pub(crate) fn reference_bounds_semantics(
    semantics: hiroute_domain::UsageSemanticsV1,
    raw: &RawUsage,
    quote: &FrozenPriceQuoteV1,
) -> Option<(u64, u64)> {
    let exact = amount_only(quote, &reference_usage_semantics(semantics, raw, quote));
    if exact.coverage == MetricCoverage::Complete {
        let amount = exact.known_sum_micros?;
        return Some((amount, amount));
    }
    if raw.write.is_some()
        || semantics.frame_kind == UsageFrameKindV1::Unknown
        || semantics.input != InputUsageMeaningV1::IncludesExclusiveCache
        || !semantics.cache_buckets_exclusive
    {
        return None;
    }
    let remaining = raw.input?.checked_sub(raw.read?)?;
    let mut scenario = raw.clone();
    let mut amount_for = |write| {
        scenario.write = Some(write);
        let result = amount_only(
            quote,
            &reference_usage_semantics(semantics, &scenario, quote),
        );
        (result.coverage == MetricCoverage::Complete)
            .then_some(result.known_sum_micros)
            .flatten()
    };
    let zero = amount_for(0)?;
    let all = amount_for(remaining)?;
    Some((zero.min(all), zero.max(all)))
}
