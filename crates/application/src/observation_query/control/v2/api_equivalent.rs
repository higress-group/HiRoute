//! Capture one current source-price generation for an explicitly requested estimate.
use super::*;
use hiroute_application_api::{ApiEquivalentValueOptionsV1, PriceTargetLocatorV1};
use hiroute_domain::{
    ApiEquivalentPriceV1, ApiEquivalentValueReportV1, PriceBillingContextV1, PriceValuationKindV1,
};
impl ObservationControl {
    pub(super) fn api_equivalent_value(
        &self,
        reader: &ObservationReaderContext,
        options: ApiEquivalentValueOptionsV1,
        now: i64,
    ) -> Result<ApiEquivalentValueReportV1, ObservationQueryError> {
        let q = options.report;
        if q.group_by != hiroute_domain::ValueGroupByV1::None
            || q.from_ms.is_some() && q.period.is_some()
        {
            return Err(ObservationQueryError::InvalidQuery);
        }
        let to_ms = q.to_ms.unwrap_or(now);
        let from_ms = q
            .from_ms
            .unwrap_or(match q.period.unwrap_or(ValuePeriodV1::SevenDays) {
                ValuePeriodV1::Today => {
                    self.clock.local_day_start_ms(to_ms).map_err(map_control)?
                }
                ValuePeriodV1::SevenDays => to_ms.saturating_sub(7 * DAY_MILLIS).saturating_add(1),
                ValuePeriodV1::ThirtyDays => to_ms.saturating_sub(30 * DAY_MILLIS),
            });
        let currency = q.currency.ok_or(ObservationQueryError::InvalidQuery)?;
        if from_ms < 0 || from_ms >= to_ms || to_ms > now {
            return Err(ObservationQueryError::InvalidQuery);
        }
        let facts = self
            .prices
            .as_ref()
            .ok_or(ObservationQueryError::Unavailable)?
            .price_control_facts()
            .map_err(map_control)?;
        if facts.source_bindings.len() > 4096 {
            return Err(ObservationQueryError::Unavailable);
        }
        let mut prices = Vec::new();
        for binding in &facts.source_bindings {
            let locator = PriceTargetLocatorV1::Binding {
                binding_id: binding.binding_id.clone(),
            };
            let (target, _, _) = crate::prices::resolve_price_target(
                &facts,
                &locator,
                &currency,
                PriceValuationKindV1::ApiEquivalent,
            )
            .map_err(|_| ObservationQueryError::InvalidQuery)?;
            let quote = facts
                .effective
                .freeze_price(
                    &target,
                    facts.evaluated_at,
                    PriceBillingContextV1::StandardTokens,
                )
                .map_err(|_| ObservationQueryError::InvalidQuery)?;
            prices.push(ApiEquivalentPriceV1 {
                binding_id: binding.binding_id.clone(),
                execution_model_configuration_id: binding.execution_model_configuration_id.clone(),
                quote,
            });
        }
        let reference = prices
            .iter()
            .find(|p| p.binding_id == options.reference_binding_id)
            .ok_or(ObservationQueryError::NotFound)?;
        self.service.port.observed_api_equivalent_value(
            reader,
            &hiroute_domain::ObservationValueQueryV2 {
                from_ms,
                to_ms,
                session_id: q.session_id.map(|s| s.to_string()),
                plan_id: q.agent_plan_id.map(|p| p.as_str().to_owned()),
                currency: Some(currency),
            },
            reference,
            &prices,
            now,
        )
    }
}
