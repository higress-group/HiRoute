use super::*;
use hiroute_domain::{ExecutionPricingEvidenceV1, FrozenPriceQuoteV1, TokenRateV1};

pub fn value_attempt(
    quote: &FrozenPriceQuoteV1,
    usage: &UsageBucketsV2,
) -> Result<AttemptValuationV2, ValuationError> {
    validate_quote(quote)?;
    let amount = amount_only(quote, usage);
    let components = [
        ("input_uncached", usage.input_uncached),
        ("output", usage.output),
        ("cache_read", usage.cache_read),
        ("cache_write", usage.cache_write),
    ]
    .into_iter()
    .zip(amount.missing)
    .map(|((bucket, tokens), missing)| ValuationComponentV2 {
        bucket: bucket.into(),
        tokens,
        missing,
    })
    .collect();
    Ok(AttemptValuationV2 {
        algorithm: VALUATION_ALGORITHM.into(),
        currency: quote.currency.clone(),
        valuation_kind: quote.valuation_kind,
        quote: quote.clone(),
        known_sum_micros: amount.known_sum_micros,
        coverage: amount.coverage,
        components,
    })
}

/// Value the recorded attempt using only its frozen quote and reported usage.
/// Equal-rate buckets can establish an amount even when their individual token
/// counts are unknown. Preserve the original component evidence in the result.
pub(crate) fn value_recorded_attempt(
    pricing: &ExecutionPricingEvidenceV1,
    raw: &RawUsage,
) -> Result<AttemptValuationV2, ValuationError> {
    let quote = pricing
        .quote
        .as_ref()
        .ok_or(ValuationError::InvalidEvidence)?;
    let observed = worker::normalize(pricing, raw);
    let mut result = value_attempt(quote, &observed)?;
    let combined = reference_usage(pricing, raw, quote);
    if combined != observed {
        let amount = amount_only(quote, &combined);
        result.algorithm = EQUAL_RATE_VALUATION_ALGORITHM.into();
        result.known_sum_micros = amount.known_sum_micros;
        result.coverage = amount.coverage;
    }
    Ok(result)
}

pub(crate) fn validate_quote(quote: &FrozenPriceQuoteV1) -> Result<(), ValuationError> {
    quote
        .verify_digest()
        .map_err(|_| ValuationError::InvalidEvidence)?;
    quote
        .exact_target
        .validate()
        .map_err(|_| ValuationError::InvalidEvidence)?;
    if quote.currency != quote.exact_target.currency
        || quote.valuation_kind != quote.exact_target.valuation_kind
        || quote.actual_source_ref != quote.exact_target.source_id
    {
        return Err(ValuationError::InvalidEvidence);
    }
    Ok(())
}

/// Shared integer evaluator for already validated quotes. Query captures verify
/// each immutable quote once; this hot path neither rehashes nor clones it.
pub(crate) struct NumericAmount {
    pub known_sum_micros: Option<u64>,
    pub coverage: MetricCoverage,
    pub missing: [Option<ValuationMissingReason>; 4],
}

pub(crate) fn amount_only(quote: &FrozenPriceQuoteV1, usage: &UsageBucketsV2) -> NumericAmount {
    let mut numerator = Some(0u128);
    let mut known = 0;
    let known_base = matches!(quote.rates.input_uncached, TokenRateV1::Known { .. })
        && matches!(quote.rates.output, TokenRateV1::Known { .. });
    let mut missing_components = [None; 4];
    for (index, (_, tokens, rate)) in [
        (
            "input_uncached",
            usage.input_uncached,
            &quote.rates.input_uncached,
        ),
        ("output", usage.output, &quote.rates.output),
        ("cache_read", usage.cache_read, &quote.rates.cache_read),
        ("cache_write", usage.cache_write, &quote.rates.cache_write),
    ]
    .into_iter()
    .enumerate()
    {
        let missing = match (tokens, rate) {
            (None, _) => Some(ValuationMissingReason::UsageUnknown),
            (Some(0), _) if known_base => {
                known += 1;
                None
            }
            (
                Some(tokens),
                TokenRateV1::Known {
                    micros_per_million_tokens,
                },
            ) => {
                known += 1;
                numerator = numerator.and_then(|sum| {
                    u128::from(tokens)
                        .checked_mul(u128::from(*micros_per_million_tokens))?
                        .checked_add(sum)
                });
                None
            }
            (Some(_), TokenRateV1::Unknown { .. }) => Some(ValuationMissingReason::PriceUnknown),
        };
        missing_components[index] = missing;
    }
    let amount = numerator
        .and_then(|value| value.checked_add(500_000))
        .map(|value| value / 1_000_000)
        .and_then(|value| u64::try_from(value).ok());
    let coverage = if amount.is_none() {
        for missing in &mut missing_components {
            if missing.is_none() {
                *missing = Some(ValuationMissingReason::ArithmeticOverflow);
            }
        }
        MetricCoverage::Unknown
    } else if known == 4 {
        MetricCoverage::Complete
    } else if known == 0 {
        MetricCoverage::Unknown
    } else {
        MetricCoverage::Partial
    };
    NumericAmount {
        known_sum_micros: if known == 0 { None } else { amount },
        coverage,
        missing: missing_components,
    }
}
