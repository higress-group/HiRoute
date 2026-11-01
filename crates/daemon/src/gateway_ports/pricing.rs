//! Composition adapter over the single control-owned price slot. Targets must
//! come from verified source materialization; binding ids alone are insufficient.
use hiroute_application::prices::{PriceSnapshotHandle, PriceSnapshotSlot};
use hiroute_domain::{
    EXECUTION_PRICING_SCHEMA_V1, ExecutionPricingEvidenceV1, PriceModelIdentityV1, PriceOriginV1,
    PriceTargetV1, PriceUnknownReasonV1, PriceValuationKindV1, TokenRateV1, WorkspaceId,
};
use hiroute_gateway::server::core_runtime::observation::{
    RequestPriceSnapshot, RequestPriceSource,
};
use hiroute_gateway::server::request_plan::RequestPriceBindingV1;
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Debug)]
struct VerifiedAttemptPriceTarget {
    pub usage_semantics: hiroute_domain::UsageSemanticsV1,
    pub target: PriceTargetV1,
    pub billing_context: hiroute_domain::PriceBillingContextV1,
    pub actual_offer_ref: Arc<str>,
}

pub struct GatewayRequestPriceSource {
    slot: Arc<PriceSnapshotSlot>,
}

impl GatewayRequestPriceSource {
    pub fn new(slot: Arc<PriceSnapshotSlot>) -> Self {
        Self { slot }
    }
}

impl RequestPriceSource for GatewayRequestPriceSource {
    fn capture_request(
        &self,
        workspace_id: &str,
        bindings: &[RequestPriceBindingV1],
        captured_at_ms: i64,
    ) -> Arc<dyn RequestPriceSnapshot> {
        // Exactly one atomic price-generation read per new LogicalRequest. V is
        // already the request's pinned publication projection; it is copied
        // into this request and never replaced by a newer publication.
        let targets = request_targets(workspace_id, bindings);
        Arc::new(CapturedPrice {
            snapshot: self.slot.capture_current_price_snapshot(),
            targets: targets.as_ref().cloned().unwrap_or_default(),
            bindings_valid: targets.is_ok(),
            captured_at_ms,
        })
    }
}

fn request_targets(
    workspace_id: &str,
    bindings: &[RequestPriceBindingV1],
) -> Result<
    BTreeMap<(String, String, String), VerifiedAttemptPriceTarget>,
    hiroute_domain::ComputeContractError,
> {
    if bindings.len() > 16_384 {
        return Err(hiroute_domain::ComputeContractError::InvalidPrice);
    }
    let workspace_id = WorkspaceId::parse(workspace_id)
        .map_err(|_| hiroute_domain::ComputeContractError::InvalidPrice)?;
    let mut targets = BTreeMap::new();
    for binding in bindings {
        let target = PriceTargetV1 {
            workspace_id: workspace_id.clone(),
            source_id: binding.source_id.to_string(),
            source_identity_digest: binding.source_identity_digest.clone(),
            model_identity: PriceModelIdentityV1::CatalogModel(
                binding.model_configuration_id.to_string(),
            ),
            currency: "USD".into(),
            valuation_kind: PriceValuationKindV1::UsageEstimate,
        };
        target.validate()?;
        if binding.actual_offer_ref.is_empty() {
            return Err(hiroute_domain::ComputeContractError::InvalidPrice);
        }
        let key = (
            binding.stable_binding_id.to_string(),
            binding.model_configuration_id.to_string(),
            binding.profile_digest.to_string(),
        );
        if targets
            .insert(
                key,
                VerifiedAttemptPriceTarget {
                    usage_semantics: binding.usage_semantics,
                    target,
                    billing_context: binding.billing_context.clone(),
                    actual_offer_ref: Arc::clone(&binding.actual_offer_ref),
                },
            )
            .is_some()
        {
            return Err(hiroute_domain::ComputeContractError::DuplicateIdentity);
        }
    }
    Ok(targets)
}

struct CapturedPrice {
    snapshot: PriceSnapshotHandle,
    targets: BTreeMap<(String, String, String), VerifiedAttemptPriceTarget>,
    bindings_valid: bool,
    captured_at_ms: i64,
}

impl RequestPriceSnapshot for CapturedPrice {
    fn freeze_attempt(
        &self,
        binding: &str,
        model: &str,
        profile: &str,
        at_ms: i64,
    ) -> ExecutionPricingEvidenceV1 {
        self.freeze_currency(binding, model, profile, at_ms, "USD")
    }
    fn freeze_spending_attempt(
        &self,
        binding: &str,
        model: &str,
        profile: &str,
        at_ms: i64,
        currency: &str,
    ) -> Option<ExecutionPricingEvidenceV1> {
        Some(self.freeze_currency(binding, model, profile, at_ms, currency))
    }
}
impl CapturedPrice {
    fn freeze_currency(
        &self,
        binding: &str,
        model: &str,
        profile: &str,
        at_ms: i64,
        currency: &str,
    ) -> ExecutionPricingEvidenceV1 {
        let mut result = ExecutionPricingEvidenceV1 {
            usage_semantics: Default::default(),
            schema_version: EXECUTION_PRICING_SCHEMA_V1.into(),
            request_generation: self.snapshot.generation_ref().cloned(),
            captured_at_ms: self.captured_at_ms,
            attempt_execution_at_ms: at_ms,
            quote: None,
            reference_quote: None,
            unknown_reason: Some(PriceUnknownReasonV1::UnverifiedOfferMapping),
        };
        let key = (binding.to_owned(), model.to_owned(), profile.to_owned());
        let Some(target) = self.targets.get(&key).filter(|_| self.bindings_valid) else {
            return result;
        };
        if currency.len() != 3 || !currency.bytes().all(|b| b.is_ascii_uppercase()) {
            return result;
        }
        let mut priced_target = target.target.clone();
        priced_target.currency = currency.into();
        result.usage_semantics = target.usage_semantics;
        // The snapshot API uses Unix seconds; execution evidence preserves ms.
        match self.snapshot.freeze_execution_price(
            &priced_target,
            binding,
            at_ms / 1000,
            target.billing_context.clone(),
        ) {
            Ok(quote)
                if quote.generation_ref.is_none()
                    || quote.origin == PriceOriginV1::Manual
                    || quote.actual_offer_ref.as_deref()
                        == Some(target.actual_offer_ref.as_ref())
                    || quote
                        .unknown_reasons
                        .contains(&PriceUnknownReasonV1::TargetNotInSnapshot) =>
            {
                result.unknown_reason = quote.unknown_reasons.first().copied();
                result.quote = Some(quote);
            }
            Ok(_) => {
                // The source/model identity is verified even when its catalog offer is not.
                // Retain that identity and usage semantics without borrowing any catalog rates,
                // so a later explicit current-price comparison can use source-specific prices.
                if let Ok(mut unknown) =
                    PriceSnapshotHandle::Unavailable(PriceUnknownReasonV1::UnverifiedOfferMapping)
                        .freeze_price(&priced_target, at_ms / 1000, target.billing_context.clone())
                {
                    unknown.generation_ref = result.request_generation.clone();
                    if let Ok(digest) = unknown.computed_digest() {
                        unknown.quote_digest = digest;
                        result.quote = Some(unknown);
                    }
                }
            }
            Err(_) => {
                result.unknown_reason = Some(PriceUnknownReasonV1::UnverifiedOfferMapping);
            }
        }
        let usable = |quote: &hiroute_domain::FrozenPriceQuoteV1| {
            matches!(quote.rates.input_uncached, TokenRateV1::Known { .. })
                && matches!(quote.rates.output, TokenRateV1::Known { .. })
        };
        if result.quote.as_ref().is_none_or(|quote| !usable(quote)) {
            let mut reference = priced_target.clone();
            reference.valuation_kind = PriceValuationKindV1::ApiEquivalent;
            if let Ok(quote) = self.snapshot.freeze_execution_price(
                &reference,
                binding,
                at_ms / 1000,
                target.billing_context.clone(),
            ) && usable(&quote)
            {
                // This is an explicitly configured reference estimate, never a
                // substitute subscription bill or a repricing of old receipts.
                result.unknown_reason = quote.unknown_reasons.first().copied();
                result.quote = Some(quote);
            }
        }
        result
    }
}

#[cfg(test)]
mod tests;
