use super::*;
use hiroute_application_api::PriceTargetLocatorV1;
use hiroute_domain::*;
use std::collections::BTreeSet;

pub(in crate::observation_query::control) enum AnalysisPriceCapture {
    Ready(AnalysisPricesV1, String),
    CurrencyChoices(Vec<String>),
}

impl ObservationControl {
    pub(super) fn analysis_read(
        &self,
        reader: &ObservationReaderContext,
        mut q: ObservationAnalysisQueryV1,
        range: Option<&AnalysisTimeRangeV1>,
        now: i64,
    ) -> Result<ObservationAnalysisPageV1, ObservationQueryError> {
        let default_basis = if q.view == AnalysisViewV1::SavingsSummary {
            AnalysisCostBasisV1::ApiEquivalent
        } else {
            AnalysisCostBasisV1::Recorded
        };
        // A fresh current-price query already supplies its capture intent. Keep
        // scope creation and evaluation in one reader/transaction, rather than
        // opening a second cold reader within the same total 1 s budget.
        if q.analysis_scope.is_none()
            && q.cursor.is_none()
            && q.view.monetary()
            && q.basis.unwrap_or(default_basis) == AnalysisCostBasisV1::ApiEquivalent
            && reader.allowed_runs().is_none()
        {
            let required = AnalysisPriceCaptureRequestV1 {
                currency: q.currency.clone(),
                plan_id: q.plan_id.clone(),
                reference_binding_id: q.reference_binding_id.clone(),
            };
            return match self.analysis_prices(reader, required, now)? {
                AnalysisPriceCapture::Ready(capture, currency) => {
                    q.currency = Some(currency);
                    self.service
                        .observation_analysis(reader, &q, range, Some(&capture), now)
                }
                AnalysisPriceCapture::CurrencyChoices(currencies) => {
                    let mut page = self
                        .service
                        .observation_analysis(reader, &q, range, None, now)?;
                    if page.price_capture_request.take().is_some() {
                        page.choices.push(AnalysisChoiceV1 {
                            field: "currency".into(), values: currencies,
                            reason: "Choose a saved API-equivalent currency; add source-specific API prices first if no currency is available.".into(),
                        });
                    }
                    Ok(page)
                }
            };
        }
        let mut page = self
            .service
            .observation_analysis(reader, &q, range, None, now)?;
        let Some(required) = page.price_capture_request.take() else {
            return Ok(page);
        };
        let (capture, currency) = match self.analysis_prices(reader, required, now)? {
            AnalysisPriceCapture::Ready(capture, currency) => (capture, currency),
            AnalysisPriceCapture::CurrencyChoices(currencies) => {
                page.choices.push(AnalysisChoiceV1 {
                    field: "currency".into(), values: currencies,
                    reason: "Choose a saved API-equivalent currency; add source-specific API prices first if no currency is available.".into(),
                });
                return Ok(page);
            }
        };
        q.analysis_scope = page.analysis_scope;
        q.cursor = None;
        q.time = Default::default();
        q.currency = Some(currency);
        self.service
            .observation_analysis(reader, &q, None, Some(&capture), now)
    }

    pub(in crate::observation_query::control) fn analysis_prices(
        &self,
        reader: &ObservationReaderContext,
        required: AnalysisPriceCaptureRequestV1,
        now: i64,
    ) -> Result<AnalysisPriceCapture, ObservationQueryError> {
        reader.check(now, false, false)?;
        let deadline = reader
            .query_deadline()
            .ok_or(ObservationQueryError::InvalidQuery)?;
        let result = self
            .prices
            .as_ref()
            .ok_or(ObservationQueryError::Unavailable)?
            .try_price_control_facts(deadline);
        reader.check(now, false, false)?;
        let facts = match result.map_err(map_control)? {
            crate::control::ControlReadAttempt::Ready(facts) => facts,
            crate::control::ControlReadAttempt::Busy => {
                return Err(ObservationQueryError::ReaderBusy);
            }
        };
        if facts.source_bindings.len() > 4096 {
            return Err(ObservationQueryError::Unavailable);
        }
        let currencies: BTreeSet<_> = facts
            .entries
            .iter()
            .filter(|e| e.target.valuation_kind == PriceValuationKindV1::ApiEquivalent)
            .map(|e| e.target.currency.clone())
            .collect();
        let currency = required.currency.or_else(|| {
            (currencies.len() == 1)
                .then(|| currencies.first().cloned())
                .flatten()
        });
        let Some(currency) = currency else {
            return Ok(AnalysisPriceCapture::CurrencyChoices(
                currencies.into_iter().collect(),
            ));
        };
        let mut prices = Vec::with_capacity(facts.source_bindings.len());
        for binding in &facts.source_bindings {
            reader.check(now, false, false)?;
            let (target, _, _) = crate::prices::resolve_price_target(
                &facts,
                &PriceTargetLocatorV1::Binding {
                    binding_id: binding.binding_id.clone(),
                },
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
        let reference = if let Some(id) = required.reference_binding_id {
            Some(id)
        } else {
            self.sole_primary(required.plan_id.as_deref(), reader, now)?
        };
        let capture = AnalysisPricesV1 {
            generation_ref: facts
                .effective
                .generation_ref()
                .map(|g| g.id.clone())
                .unwrap_or_else(|| "unavailable".into()),
            evaluated_at_ms: facts.evaluated_at.saturating_mul(1000),
            prices,
            reference_binding_id: reference,
        };
        Ok(AnalysisPriceCapture::Ready(capture, currency))
    }

    fn sole_primary(
        &self,
        plan_id: Option<&str>,
        reader: &ObservationReaderContext,
        now: i64,
    ) -> Result<Option<String>, ObservationQueryError> {
        let Some(routing) = &self.routing else {
            return Ok(None);
        };
        let result = routing.try_active_publication(
            &self.workspace_id,
            reader
                .query_deadline()
                .ok_or(ObservationQueryError::InvalidQuery)?,
        );
        reader.check(now, false, false)?;
        let publication = match result.map_err(map_control)? {
            crate::control::ControlReadAttempt::Ready(Some(publication)) => publication,
            crate::control::ControlReadAttempt::Ready(None) => return Ok(None),
            crate::control::ControlReadAttempt::Busy => {
                return Err(ObservationQueryError::ReaderBusy);
            }
        };
        let plans: Vec<_> = publication
            .plans
            .iter()
            .filter(|p| plan_id.is_none_or(|id| p.body.identity.agent_plan_id.as_str() == id))
            .collect();
        if plans.len() != 1 {
            return Ok(None);
        }
        let bindings: BTreeSet<_> = plans[0]
            .body
            .materialized
            .attempt_owned
            .groups
            .iter()
            .filter(|g| g.group_id == MaterializedGroupId::Primary)
            .flat_map(|g| g.candidates.iter().map(|c| c.binding_id.clone()))
            .collect();
        Ok((bindings.len() == 1)
            .then(|| bindings.first().cloned())
            .flatten())
    }
}
