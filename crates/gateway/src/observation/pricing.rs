//! Typed, pure-memory pricing observation port. Implementations may only acquire
//! immutable snapshots and perform bounded lookups; no I/O, control writer lock,
//! SecretStore access or async work is permitted on this request path.
use hiroute_domain::{
    EXECUTION_PRICING_SCHEMA_V1, ExecutionPricingEvidenceV1, PriceUnknownReasonV1,
};
use std::sync::Arc;

use crate::server::request_plan::RequestPriceBindingV1;

pub trait RequestPriceSnapshot: Send + Sync {
    fn freeze_spending_attempt(
        &self,
        binding: &str,
        model: &str,
        profile: &str,
        at_ms: i64,
        currency: &str,
    ) -> Option<ExecutionPricingEvidenceV1> {
        let result = self.freeze_attempt(binding, model, profile, at_ms);
        result
            .quote
            .as_ref()
            .filter(|q| q.currency == currency)
            .map(|_| result.clone())
    }

    fn freeze_attempt(
        &self,
        stable_binding_id: &str,
        model_configuration_id: &str,
        profile_digest: &str,
        attempt_execution_at_ms: i64,
    ) -> ExecutionPricingEvidenceV1;
}

pub trait RequestPriceSource: Send + Sync {
    fn capture_request(
        &self,
        workspace_id: &str,
        bindings: &[RequestPriceBindingV1],
        captured_at_ms: i64,
    ) -> Arc<dyn RequestPriceSnapshot>;
}

pub(super) struct UnavailableRequestPrice {
    pub(super) captured_at_ms: i64,
}

impl RequestPriceSnapshot for UnavailableRequestPrice {
    fn freeze_attempt(&self, _: &str, _: &str, _: &str, at_ms: i64) -> ExecutionPricingEvidenceV1 {
        ExecutionPricingEvidenceV1 {
            usage_semantics: Default::default(),
            schema_version: EXECUTION_PRICING_SCHEMA_V1.into(),
            request_generation: None,
            captured_at_ms: self.captured_at_ms,
            attempt_execution_at_ms: at_ms,
            quote: None,
            reference_quote: None,
            unknown_reason: Some(PriceUnknownReasonV1::SnapshotUnavailable),
        }
    }
}

pub(super) fn capture(
    source: Option<&Arc<dyn RequestPriceSource>>,
    workspace: &str,
    bindings: &[RequestPriceBindingV1],
    at_ms: i64,
) -> Arc<dyn RequestPriceSnapshot> {
    source
        .and_then(|source| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                source.capture_request(workspace, bindings, at_ms)
            }))
            .ok()
        })
        .unwrap_or_else(|| {
            Arc::new(UnavailableRequestPrice {
                captured_at_ms: at_ms,
            })
        })
}
