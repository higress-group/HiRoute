//! Amount-only normalization for an explicitly selected current-price comparison.
//! Equal-rate buckets can be combined algebraically; stored usage is never filled in.
use super::{RawUsage, UsageBucketsV2, normalize};
use hiroute_domain::{
    ExecutionPricingEvidenceV1, FrozenPriceQuoteV1, InputUsageMeaningV1, TokenRateV1,
    UsageFrameKindV1,
};

pub(crate) fn reference_usage(
    pricing: &ExecutionPricingEvidenceV1,
    raw: &RawUsage,
    quote: &FrozenPriceQuoteV1,
) -> UsageBucketsV2 {
    let mut usage = normalize(pricing, raw);
    let semantics = pricing.usage_semantics;
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
