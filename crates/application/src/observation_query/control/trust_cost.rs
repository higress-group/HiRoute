use super::*;
use hiroute_domain::*;

impl ObservationControl {
    pub(super) fn trust_cost(
        &self,
        reader: &ObservationReaderContext,
        query: &PlanTrustQueryV1,
        revision: Option<u64>,
        from_ms: i64,
        to_ms: i64,
        now: i64,
    ) -> Value {
        let result = (|| {
            let bindings = self
                .state
                .trust_primary_bindings(
                    &self.workspace_id,
                    &query.plan_id,
                    revision,
                    query.branch_id.as_deref(),
                )
                .map_err(|_| "reference_unavailable")?;
            let [binding] = bindings.as_slice() else {
                return Err("primary_not_unique");
            };
            let reader = reader.clone().with_query_deadline(
                std::time::Instant::now() + hiroute_domain::ObservationReaderContext::QUERY_BUDGET,
            );
            let capture = self
                .analysis_prices(
                    &reader,
                    AnalysisPriceCaptureRequestV1 {
                        plan_id: Some(query.plan_id.clone()),
                        reference_binding_id: Some(binding.clone()),
                        currency: None,
                    },
                    now,
                )
                .map_err(|_| "prices_unavailable")?;
            let v2::analysis::AnalysisPriceCapture::Ready(prices, currency) = capture else {
                return Err("currency_not_unique");
            };
            let reference = prices
                .prices
                .iter()
                .find(|p| p.binding_id == *binding)
                .ok_or("reference_unavailable")?;
            let report =
                self.service
                    .port
                    .observed_trust_cost(
                        &reader,
                        &TrustCostScopeV1 {
                            plan_id: query.plan_id.clone(),
                            revision,
                            branch_id: query.branch_id.clone(),
                            group: ExecutionGroupV1::Regular,
                            from_ms,
                            to_ms,
                            currency,
                        },
                        reference,
                        &prices.prices,
                        now,
                    )
                    .map_err(|error| match error {
                        ObservationQueryError::ScopeTooLarge
                        | ObservationQueryError::ReadTimeout => "scope_too_large",
                        ObservationQueryError::ReaderBusy => "reader_busy",
                        _ => "projection_or_evidence_unavailable",
                    })?;
            let difference = report.estimated_savings_micros.and_then(i64::checked_neg);
            Ok(
                json!({"kind":"available","scope":{"plan_id":query.plan_id,"plan_revision":revision,"branch_id":query.branch_id,"execution_group":"regular"},"difference_micros":difference,"comparison":report}),
            )
        })();
        result.unwrap_or_else(|reason| json!({"kind":"unavailable","reason":reason}))
    }
}
