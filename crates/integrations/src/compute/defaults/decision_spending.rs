//! Decision billing is scoped to its real endpoint, codec and model, independently
//! of an ordinary inference binding. Build the index while admitting the catalog.
use hiroute_domain::*;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub(crate) struct DecisionSpendingIndex {
    prices: BTreeMap<(String, String), Vec<(String, DocumentedModelPriceV1)>>,
    generation: Option<PriceGenerationRefV1>,
}

impl DecisionSpendingIndex {
    pub(crate) fn build(
        metadata: &ModelMetadataCatalogV1,
        prices: &[DocumentedModelPriceV1],
        revision: u64,
        digest: CanonicalDigest,
    ) -> Self {
        let interfaces: BTreeMap<_, _> = metadata
            .access_products
            .iter()
            .flat_map(|p| &p.interfaces)
            .map(|i| (i.interface_key.as_str(), i))
            .collect();
        let mut result = Self {
            generation: Some(PriceGenerationRefV1 {
                id: format!("decision-price/{}", &digest.as_str()[7..]),
                digest: digest.clone(),
                configuration_revision: revision,
                catalog_refs: vec![PriceFactRefV1 {
                    id: "official-model-data".into(),
                    revision,
                    digest,
                }],
            }),
            ..Self::default()
        };
        for binding in &metadata.endpoint_bindings {
            for constraint in &binding.decision_constraints {
                let Some(interface) = interfaces.get(constraint.interface_key.as_str()) else {
                    continue;
                };
                // The provider's current Decisions billing contract is input-only.
                // https://developers.openai.com/api/docs/guides/decisions#pricing-and-availability
                // Input/cache rates and thresholds come from the single metadata catalog.
                if interface.protocol != "openai-decisions"
                    || binding.upstream_model_id != "gpt-6-luna"
                {
                    continue;
                }
                let Some(endpoint) = interface
                    .base_url
                    .as_deref()
                    .zip(interface.request_path.as_deref())
                    .and_then(|(base, path)| reqwest::Url::parse(&format!("{base}{path}")).ok())
                else {
                    continue;
                };
                if endpoint.as_str() != "https://api.openai.com/v1/decisions" {
                    continue;
                }
                let matched = prices
                    .iter()
                    .filter(|price| {
                        price.product_key == binding.product_key
                            && price.model_key == binding.model_key
                            && price.upstream_model_id == binding.upstream_model_id
                    })
                    .map(|price| (binding.binding_key.clone(), price.clone()))
                    .collect();
                result.prices.insert(
                    (endpoint.to_string(), binding.upstream_model_id.clone()),
                    matched,
                );
            }
        }
        result
    }

    pub(crate) fn estimates(
        &self,
        binding: &DecisionCredentialBindingV1,
        input_bound: u64,
        at_ms: i64,
    ) -> BTreeMap<String, KeySpendEstimateV1> {
        let mut output = BTreeMap::new();
        if binding.validate().is_err()
            || binding.wire_protocol != DecisionWireProtocolV2::OpenaiDecisions
            || input_bound == 0
            || at_ms < 0
        {
            return output;
        }
        let Some(endpoint) = reqwest::Url::parse(&binding.endpoint).ok() else {
            return output;
        };
        let Some(prices) = self
            .prices
            .get(&(endpoint.to_string(), binding.model.clone()))
        else {
            return output;
        };
        let mut currencies = BTreeMap::<&str, Vec<_>>::new();
        for price in prices {
            currencies.entry(&price.1.currency).or_default().push(price);
        }
        for (currency, prices) in currencies {
            let [price] = prices.as_slice() else { continue };
            if let Some(estimate) = self.estimate(binding, input_bound, at_ms, &price.0, &price.1) {
                output.insert(currency.to_owned(), estimate);
            }
        }
        output
    }

    fn estimate(
        &self,
        binding: &DecisionCredentialBindingV1,
        input_bound: u64,
        at_ms: i64,
        metadata_binding: &str,
        price: &DocumentedModelPriceV1,
    ) -> Option<KeySpendEstimateV1> {
        if price.unit != "per-1m-tokens" || price.price_state != "exact" {
            return None;
        }
        let tiers = input_tiers(price)?;
        let generation = self.generation.clone()?;
        let model =
            CanonicalDigest::of(&(&binding.endpoint, binding.wire_protocol, &binding.model))
                .ok()?;
        let target = PriceTargetV1 {
            workspace_id: WorkspaceId::default(),
            source_id: binding.source_id.clone(),
            source_identity_digest: binding.source_lineage_digest.clone(),
            model_identity: PriceModelIdentityV1::LocalModel(format!(
                "decision/{}",
                &model.as_str()[7..]
            )),
            currency: price.currency.clone(),
            valuation_kind: PriceValuationKindV1::UsageEstimate,
        };
        let mut quote = FrozenPriceQuoteV1 {
            generation_ref: Some(generation.clone()),
            exact_target: target,
            actual_source_ref: binding.source_id.clone(),
            actual_offer_ref: Some(metadata_binding.into()),
            reference_model_offer_ref: None,
            valuation_kind: PriceValuationKindV1::UsageEstimate,
            currency: price.currency.clone(),
            unit: PriceUnitV1::MicrosPerMillionTokens,
            rates: tiers.first()?.rates.clone(),
            origin: PriceOriginV1::Catalog,
            applied_override_refs: vec![],
            selected_rule_refs: vec![PriceFactRefV1 {
                id: format!("decision-input/{}", price.pricing_key),
                revision: generation.configuration_revision,
                digest: CanonicalDigest::of(&(price, "openai-decisions-input-only/v1")).ok()?,
            }],
            attempt_execution_at: at_ms / 1000,
            applied_schedule: None,
            billing_context: PriceBillingContextV1::StandardTokens,
            unknown_reasons: vec![],
            quote_digest: CanonicalDigest::of_bytes(b"pending"),
        };
        quote.quote_digest = quote.computed_digest().ok()?;
        let estimate = KeySpendEstimateV1 {
            pricing: ExecutionPricingEvidenceV1 {
                usage_semantics: UsageSemanticsV1 {
                    frame_kind: UsageFrameKindV1::Cumulative,
                    input: InputUsageMeaningV1::IncludesExclusiveCache,
                    output: OutputUsageMeaningV1::IncludesReasoning,
                    cache_buckets_exclusive: true,
                },
                schema_version: EXECUTION_PRICING_SCHEMA_V1.into(),
                request_generation: Some(generation),
                captured_at_ms: at_ms,
                attempt_execution_at_ms: at_ms,
                quote: Some(quote),
                reference_quote: None,
                unknown_reason: None,
            },
            input_bound,
            output_bound: 0,
            input_price_tiers: tiers,
        };
        estimate.upper_bound_micros()?;
        Some(estimate)
    }
}

fn input_tiers(price: &DocumentedModelPriceV1) -> Option<Vec<KeySpendInputPriceTierV1>> {
    if price.schedules.is_empty() || price.schedules.len() > 8 {
        return None;
    }
    let mut ranges = Vec::new();
    for schedule in &price.schedules {
        if schedule.application != "whole-request" {
            return None;
        }
        let condition = &schedule.condition;
        let kind = condition.get("kind")?.as_str()?;
        let (start, end) = match kind {
            "unconditional" if condition.len() == 1 => (0, None),
            "prompt-token-threshold"
                if condition.get("service_tier")?.as_str()? == "standard"
                    && condition.get("processing")?.as_str()? == "non-regional"
                    && condition.keys().all(|k| {
                        matches!(
                            k.as_str(),
                            "kind"
                                | "service_tier"
                                | "processing"
                                | "minimum_input_tokens"
                                | "maximum_input_tokens"
                        )
                    }) =>
            {
                let start = condition
                    .get("minimum_input_tokens")
                    .map(|v| v.as_u64())
                    .unwrap_or(Some(0))?;
                let end = match condition.get("maximum_input_tokens") {
                    Some(value) => Some(value.as_u64()?),
                    None => None,
                };
                (start, end)
            }
            _ => return None,
        };
        if end.is_some_and(|end| end < start) {
            return None;
        }
        let rate = |key| TokenRateV1::parse_decimal(schedule.rates.get(key)?).ok();
        let rates = TokenRatesV1 {
            input_uncached: rate("input")?,
            output: TokenRateV1::known(0),
            cache_read: rate("cache-read")?,
            cache_write: rate("cache-write")?,
        };
        ranges.push((
            start,
            KeySpendInputPriceTierV1 {
                through_input_tokens: end,
                rates,
            },
        ));
    }
    ranges.sort_by_key(|(start, _)| *start);
    let mut expected = Some(0);
    for (start, tier) in &ranges {
        if expected != Some(*start) {
            return None;
        }
        expected = tier.through_input_tokens.and_then(|end| end.checked_add(1));
    }
    if expected.is_some() {
        return None;
    }
    Some(ranges.into_iter().map(|(_, tier)| tier).collect())
}

impl super::TrustedReleaseCatalog {
    pub fn decision_spend_estimates(
        &self,
        binding: &DecisionCredentialBindingV1,
        input_bound: u64,
        at_ms: i64,
    ) -> BTreeMap<String, KeySpendEstimateV1> {
        self.decision_spending_index
            .estimates(binding, input_bound, at_ms)
    }
    pub(crate) fn rebuild_decision_spending_index(&mut self) {
        let (metadata, prices, revision, digest) = if let Some(snapshot) = &self.official_snapshot {
            (
                &snapshot.data.metadata_catalog,
                &snapshot.data.pricing_facts,
                snapshot.data_revision,
                snapshot.data_digest.clone(),
            )
        } else {
            (
                &self.current_release_model_data.metadata_catalog,
                &self.current_release_model_data.pricing_facts,
                self.model_data_sequence,
                self.model_data_digest.clone(),
            )
        };
        self.decision_spending_index =
            DecisionSpendingIndex::build(metadata, prices, revision, digest);
    }
}

#[cfg(test)]
mod tests;
