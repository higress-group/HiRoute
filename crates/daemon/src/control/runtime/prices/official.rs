//! Convert only exact supplier/product/wire scopes and evaluable documented conditions.
use super::*;
use hiroute_domain::{DocumentedModelPriceV1, PriceUnknownReasonV1, TokenRateV1};

pub(super) fn documented_prices<'a>(
    catalog: &'a TrustedReleaseCatalog,
    binding: &PriceSourceBinding,
) -> Vec<&'a DocumentedModelPriceV1> {
    if binding.offer_ref.is_none() {
        return vec![];
    }
    let Some(resolved) = binding
        .connection_option_id
        .as_deref()
        .and_then(|id| catalog.resolve_connection_option(id).ok())
    else {
        return vec![];
    };
    let capabilities = catalog
        .model_data()
        .model_endpoint_capabilities
        .iter()
        .filter(|cap| {
            cap.model_configuration_id == binding.model_configuration_id
                && cap.connector_id == resolved.connector.connector_id
                && cap.connector_revision == resolved.connector.revision
                && cap.endpoint_profile_id == resolved.endpoint_profile.endpoint_profile_id
                && cap.endpoint_profile_revision == resolved.endpoint_profile.revision
        });
    let scopes = capabilities
        .flat_map(|cap| hiroute_integrations::compute::matching_endpoint_metadata(catalog, cap))
        .collect::<Vec<_>>();
    let Some(data) = catalog.complete_official_data() else {
        return vec![];
    };
    data.pricing_facts
        .iter()
        .filter(|price| {
            scopes.iter().any(|binding| {
                binding.product_key == price.product_key
                    && binding.model_key == price.model_key
                    && binding.upstream_model_id == price.upstream_model_id
            })
        })
        .collect()
}

fn rates(schedule: &hiroute_domain::DocumentedPriceScheduleV1) -> TokenRatesV1 {
    let rate = |keys: &[&str], absent| {
        let matches = keys
            .iter()
            .filter_map(|key| schedule.rates.get(*key))
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [] => TokenRateV1::unknown(absent),
            [value] => TokenRateV1::parse_decimal(value).unwrap_or_else(|_| {
                TokenRateV1::unknown(PriceUnknownReasonV1::UnsupportedBillingCondition)
            }),
            _ => TokenRateV1::unknown(PriceUnknownReasonV1::AmbiguousRule),
        }
    };
    TokenRatesV1 {
        input_uncached: rate(
            &["input", "input-cache-miss"],
            PriceUnknownReasonV1::PriceNotCollected,
        ),
        output: rate(&["output"], PriceUnknownReasonV1::PriceNotCollected),
        cache_read: rate(
            &["cache-read", "input-cache-hit"],
            PriceUnknownReasonV1::CacheRateNotCollected,
        ),
        cache_write: rate(
            &["cache-write"],
            PriceUnknownReasonV1::CacheRateNotCollected,
        ),
    }
}

pub(super) fn rules(
    prices: &[&DocumentedModelPriceV1],
    currency: &str,
    revision: u64,
) -> Option<Vec<CatalogPriceRuleV2>> {
    if prices.is_empty() {
        return None;
    }
    let matching = prices
        .iter()
        .filter(|p| p.currency == currency)
        .copied()
        .collect::<Vec<_>>();
    // Presence of this product's documentation is authoritative even when its currency/components
    // or conditions cannot be evaluated. Never fall through to a two-component legacy price.
    let selected = match matching.as_slice() {
        [price] => Some(*price),
        _ => None,
    };
    let known = selected.and_then(|price| match price.schedules.as_slice() {
        [schedule]
            if price.price_state == "exact"
                && schedule.application == "whole-request"
                && schedule.condition.len() == 1
                && schedule
                    .condition
                    .get("kind")
                    .and_then(serde_json::Value::as_str)
                    == Some("unconditional") =>
        {
            Some(rates(schedule))
        }
        _ => None,
    });
    let digest = CanonicalDigest::of(&matching).ok()?;
    Some(vec![CatalogPriceRuleV2 {
        rule_ref: PriceFactRefV1 {
            id: format!(
                "documented/{}",
                &CanonicalDigest::of(&matching.iter().map(|p| &p.pricing_key).collect::<Vec<_>>())
                    .ok()?
                    .as_str()[7..]
            ),
            revision,
            digest,
        },
        rates: known.unwrap_or_else(|| {
            TokenRatesV1::unknown(if matching.len() > 1 {
                PriceUnknownReasonV1::AmbiguousRule
            } else {
                PriceUnknownReasonV1::UnsupportedBillingCondition
            })
        }),
        schedule: None,
    }])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn price() -> DocumentedModelPriceV1 {
        serde_json::from_value(json!({"pricing_key":"product/model/id","product_key":"product","model_key":"model","upstream_model_id":"id","currency":"USD","unit":"per-1m-tokens","price_state":"exact","evidence_refs":["source"],"schedules":[{"schedule_key":"standard","condition":{"kind":"unconditional"},"application":"whole-request","rates":{"input":"0","output":"12.5","cache-read":"0.025","cache-write":"0.1"}}]})).unwrap()
    }
    #[test]
    fn model_prices_preserve_four_components_zero_currency_and_unsupported_conditions() {
        let mut p = price();
        let rule = rules(&[&p], "USD", 3).unwrap().remove(0);
        assert_eq!(rule.rates.input_uncached, TokenRateV1::known(0));
        assert_eq!(rule.rates.output, TokenRateV1::known(12_500_000));
        assert_eq!(rule.rates.cache_read, TokenRateV1::known(25_000));
        assert_eq!(rule.rates.cache_write, TokenRateV1::known(100_000));
        assert_ne!(rules(&[&p], "CNY", 3).unwrap()[0].rates, rule.rates);
        p.schedules[0]
            .condition
            .insert("maximum_input_tokens".into(), json!(100));
        assert_eq!(
            rules(&[&p], "USD", 3).unwrap()[0].rates,
            TokenRatesV1::unknown(PriceUnknownReasonV1::UnsupportedBillingCondition)
        );
        assert!(rules(&[], "USD", 3).is_none());
    }
}
