use super::*;
use hiroute_domain::{FrozenPriceQuoteV1, TokenRateV1};

pub fn value_attempt(
    quote: &FrozenPriceQuoteV1,
    usage: &UsageBucketsV2,
) -> Result<AttemptValuationV2, ValuationError> {
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
    let mut numerator = Some(0u128);
    let mut known = 0;
    let known_base = matches!(quote.rates.input_uncached, TokenRateV1::Known { .. })
        && matches!(quote.rates.output, TokenRateV1::Known { .. });
    let mut components = Vec::with_capacity(4);
    for (name, tokens, rate) in [
        (
            "input_uncached",
            usage.input_uncached,
            &quote.rates.input_uncached,
        ),
        ("output", usage.output, &quote.rates.output),
        ("cache_read", usage.cache_read, &quote.rates.cache_read),
        ("cache_write", usage.cache_write, &quote.rates.cache_write),
    ] {
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
        components.push(ValuationComponentV2 {
            bucket: name.into(),
            tokens,
            missing,
        });
    }
    let amount = numerator
        .and_then(|value| value.checked_add(500_000))
        .map(|value| value / 1_000_000)
        .and_then(|value| u64::try_from(value).ok());
    let coverage = if amount.is_none() {
        for component in &mut components {
            if component.missing.is_none() {
                component.missing = Some(ValuationMissingReason::ArithmeticOverflow);
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
    Ok(AttemptValuationV2 {
        algorithm: VALUATION_ALGORITHM.into(),
        currency: quote.currency.clone(),
        valuation_kind: quote.valuation_kind,
        quote: quote.clone(),
        known_sum_micros: if known == 0 { None } else { amount },
        coverage,
        components,
    })
}
