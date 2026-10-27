//! Read-time API reference comparison. Never changes frozen settlement or receipts.
use super::*;
use crate::valuation::{
    MetricCoverage, RawUsage, reference_bounds_semantics, reference_usage, value_attempt,
};
use hiroute_domain::{
    ApiEquivalentBoundsV1, ApiEquivalentPriceV1, ApiEquivalentValueReportV1,
    ExecutionPricingEvidenceV1, ObservationMetricCoverageV2, ObservationValueQueryV2,
    PriceValuationKindV1,
};
use rusqlite::{Connection, OpenFlags, params};
use std::collections::BTreeMap;

type Input = (
    String,
    bool,
    bool,
    Option<u64>,
    Option<u64>,
    Option<String>,
    Option<String>,
);
impl crate::LocalObservationStore {
    pub fn observed_api_equivalent_value(
        &self,
        reader: &ObservationReaderContext,
        q: &ObservationValueQueryV2,
        reference: &ApiEquivalentPriceV1,
        prices: &[ApiEquivalentPriceV1],
        now_ms: i64,
    ) -> Result<ApiEquivalentValueReportV1, ObservationV2Error> {
        reader.check(now_ms, false, false)?;
        if reader.allowed_runs().is_some() {
            return Err(ObservationV2Error::Unauthorized);
        }
        if q.from_ms < 0
            || q.from_ms >= q.to_ms
            || q.to_ms > now_ms
            || prices.len() > 4096
            || [&q.session_id, &q.plan_id, &q.currency]
                .into_iter()
                .flatten()
                .any(|s| !identifier(s))
        {
            return Err(ObservationV2Error::Invalid);
        }
        let currency = q.currency.as_deref().ok_or(ObservationV2Error::Invalid)?;
        for p in prices.iter().chain(std::iter::once(reference)) {
            if p.quote.currency != currency
                || p.quote.valuation_kind != PriceValuationKindV1::ApiEquivalent
                || p.quote.exact_target.workspace_id != *reader.workspace()
                || p.quote.verify_digest().is_err()
            {
                return Err(ObservationV2Error::Invalid);
            }
        }
        let _permit = self.query_permit()?;
        let mut db =
            Connection::open_with_flags(&self.activity_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let _deadline = QueryDeadline::start_for(&db, reader)?;
        // Four existing read permits bound page-cache memory to 64 MiB. Keeping
        // narrow numeric/source indexes resident avoids repeated large-row reads
        // without extending the shared deadline or retaining query results.
        db.pragma_update(None, "cache_size", -16_384)?;
        let tx = db.transaction()?;
        let visibility: u64 = tx.query_row("SELECT CAST(value AS INTEGER) FROM observation_meta WHERE key='query_visibility_generation'", [], |r| r.get(0))?;
        let retained = now_ms
            .saturating_sub(crate::managed_text::RETENTION_MS)
            .saturating_add(1);
        let comparison = if let Some(projected) = crate::analysis::projected_comparison(
            &tx,
            reader,
            q,
            reference,
            prices,
            q.from_ms.max(retained),
            now_ms,
        )? {
            projected
        } else {
            let args = params![
                reader.workspace().as_str(),
                q.from_ms.max(retained),
                q.to_ms,
                q.session_id,
                q.plan_id
            ];
            let scope = "WITH selected AS (SELECT r.* FROM logical_requests r JOIN sessions s ON s.workspace_id=r.workspace_id AND s.session_id=r.session_id WHERE r.workspace_id=?1 AND r.started_at_ms>=?2 AND r.started_at_ms<?3 AND (?4 IS NULL OR r.session_id=?4) AND (?5 IS NULL OR EXISTS(SELECT 1 FROM valuation_requests_v2 v WHERE v.workspace_id=r.workspace_id AND v.request_id=r.request_id AND v.plan_id=?5)) AND (s.tombstone_reason IS NULL OR s.content_completeness='deleted'))";
            let excluded: u64 = tx.query_row(
                &format!("{scope} SELECT COUNT(*) FROM selected WHERE traffic_kind!='normal'"),
                args,
                |r| r.get(0),
            )?;
            let mut statement = tx.prepare(&format!("{scope} SELECT r.request_id,COALESCE(v.terminal,0),COALESCE(v.partial,1),v.accepted_ordinal,i.ordinal,CASE WHEN length(i.pricing_json)<=65536 THEN i.pricing_json ELSE NULL END,CASE WHEN length(i.usage_json)<=65536 THEN i.usage_json ELSE NULL END FROM selected r LEFT JOIN valuation_requests_v2 v ON v.workspace_id=r.workspace_id AND v.request_id=r.request_id LEFT JOIN valuation_attempt_inputs_v2 i ON i.workspace_id=r.workspace_id AND i.request_id=r.request_id WHERE r.traffic_kind='normal' ORDER BY r.request_id,i.ordinal"))?;
            let mut requests = BTreeMap::<String, Comparison>::new();
            let mut used = std::collections::BTreeSet::new();
            let mut rows = statement.query(args)?;
            while let Some(row) = rows.next()? {
                let (id, terminal, partial, accepted, ordinal, pricing, usage): Input = (
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                );
                let comparison = requests.entry(id).or_insert_with(|| Comparison {
                    complete: terminal && !partial,
                    ..Default::default()
                });
                let parsed = pricing
                    .as_deref()
                    .and_then(|p| serde_json::from_str::<ExecutionPricingEvidenceV1>(p).ok());
                let raw = usage
                    .as_deref()
                    .and_then(|u| serde_json::from_str::<RawUsage>(u).ok());
                let Some((pricing, raw)) = parsed.zip(raw).filter(|(p, _)| p.validate().is_ok())
                else {
                    comparison.complete = false;
                    continue;
                };
                let price = pricing.quote.as_ref().and_then(|old| {
                prices.iter().find(|p| {
                    old.billing_context == hiroute_domain::PriceBillingContextV1::StandardTokens
                        && p.quote.exact_target.source_id == old.exact_target.source_id
                        && p.quote.exact_target.source_identity_digest
                            == old.exact_target.source_identity_digest
                        // Execution binds the exact model_configuration_id. Catalog/local
                        // is its price-index locator kind, not a different native model.
                        && (model_key(&p.quote.exact_target.model_identity) == model_key(&old.exact_target.model_identity)
                            || p.execution_model_configuration_id.as_deref() == Some(model_key(&old.exact_target.model_identity)))
                })
            });
                if let Some(price) = price {
                    used.insert(price.binding_id.clone());
                    let amount =
                        value_attempt(&price.quote, &reference_usage(&pricing, &raw, &price.quote))
                            .map_err(|_| ObservationV2Error::Invalid)?;
                    if amount.coverage != MetricCoverage::Complete {
                        comparison.complete = false;
                    }
                    comparison.actual = comparison
                        .actual
                        .and_then(|s| s.checked_add(amount.known_sum_micros?));
                } else {
                    comparison.complete = false;
                }
                if ordinal.is_some() && accepted == ordinal {
                    comparison.accepted = Some((pricing, raw));
                }
            }
            drop(rows);
            drop(statement);
            let total = requests.len() as u64;
            let mut compared = 0;
            let mut routed_full = Some(0u64);
            let mut routed_overflow = false;
            let mut reference_bounds = Some((0u64, 0u64));
            let mut actual = Some(0u64);
            let mut baseline = Some(0u64);
            for (_, comparison) in requests {
                if comparison.complete {
                    if let Some(known) = comparison.actual {
                        if let Some(sum) = routed_full {
                            routed_full = sum.checked_add(known);
                            routed_overflow |= routed_full.is_none();
                        }
                    } else {
                        routed_full = None;
                        routed_overflow = true;
                    }
                } else {
                    routed_full = None;
                }
                let reference_amount = comparison
                    .accepted
                    .as_ref()
                    .map(|(pricing, raw)| {
                        value_attempt(
                            &reference.quote,
                            &reference_usage(pricing, raw, &reference.quote),
                        )
                    })
                    .transpose()
                    .map_err(|_| ObservationV2Error::Invalid)?;
                let bounds = reference_amount
                    .as_ref()
                    .filter(|amount| amount.coverage == MetricCoverage::Complete)
                    .and_then(|amount| amount.known_sum_micros)
                    .map(|amount| (amount, amount))
                    .or_else(|| {
                        comparison.accepted.as_ref().and_then(|(pricing, raw)| {
                            reference_bounds_semantics(
                                pricing.usage_semantics,
                                raw,
                                &reference.quote,
                            )
                        })
                    });
                if comparison.complete && comparison.actual.is_some() {
                    reference_bounds = reference_bounds.zip(bounds).and_then(
                        |((lower, upper), (request_lower, request_upper))| {
                            Some((
                                lower.checked_add(request_lower)?,
                                upper.checked_add(request_upper)?,
                            ))
                        },
                    );
                } else {
                    reference_bounds = None;
                }
                if comparison.complete
                    && reference_amount
                        .as_ref()
                        .is_some_and(|a| a.coverage == MetricCoverage::Complete)
                    && comparison.actual.is_some()
                {
                    compared += 1;
                    actual = actual.and_then(|s| s.checked_add(comparison.actual?));
                    baseline =
                        baseline.and_then(|s| s.checked_add(reference_amount?.known_sum_micros?));
                }
            }
            crate::analysis::ProjectedComparison {
                total,
                compared,
                routed_full,
                routed_overflow,
                reference_bounds,
                actual,
                baseline,
                used,
                excluded,
            }
        };
        tx.commit()?;
        check_visibility(&db, visibility)?;
        reader.check(now_ms, false, false)?;
        Ok(comparison_report(
            q, reference, prices, now_ms, retained, comparison,
        ))
    }
}

pub(super) fn comparison_report(
    q: &ObservationValueQueryV2,
    reference: &ApiEquivalentPriceV1,
    prices: &[ApiEquivalentPriceV1],
    now_ms: i64,
    retained: i64,
    comparison: crate::analysis::ProjectedComparison,
) -> ApiEquivalentValueReportV1 {
    let crate::analysis::ProjectedComparison {
        total,
        compared,
        routed_full,
        routed_overflow,
        reference_bounds,
        actual,
        baseline,
        used,
        excluded,
    } = comparison;
    let currency = q.currency.as_deref().expect("admitted currency");
    let retention = q.from_ms < retained;
    let complete =
        total > 0 && compared == total && !retention && actual.is_some() && baseline.is_some();
    let difference = if complete {
        baseline
            .zip(actual)
            .and_then(|(b, a)| i64::try_from(i128::from(b) - i128::from(a)).ok())
    } else {
        None
    };
    let overflow = routed_overflow
        || (compared > 0 && (actual.is_none() || baseline.is_none()))
        || (complete && difference.is_none());
    let same_usage_bounds = (!complete && total > 0 && !retention && !overflow)
        .then(|| {
            let routed = routed_full?;
            let (lower, upper) = reference_bounds?;
            Some(ApiEquivalentBoundsV1 {
                reference_min_micros: lower,
                reference_max_micros: upper,
                difference_min_micros: i64::try_from(i128::from(routed) - i128::from(upper))
                    .ok()?,
                difference_max_micros: i64::try_from(i128::from(routed) - i128::from(lower))
                    .ok()?,
            })
        })
        .flatten();
    ApiEquivalentValueReportV1 {
            schema: "hiroute.api-equivalent-value/v1".into(),
            from_ms: q.from_ms,
            to_ms: q.to_ms,
            evaluated_at_ms: now_ms,
            currency: currency.into(),
            reference: reference.clone(),
            prices: prices.iter().filter(|p| used.contains(&p.binding_id)).cloned().collect(),
            request_count: total,
            compared_request_count: compared,
            arithmetic_overflow: overflow,
            missing_request_count: total-compared,
            excluded_request_count: excluded,
            retention_boundary_partial: retention,
            coverage: if complete && !overflow {
                ObservationMetricCoverageV2::Complete
            } else if compared>0 {
                ObservationMetricCoverageV2::Partial
            } else {
                ObservationMetricCoverageV2::Unknown
            },
            routed_estimate_micros: (total>0 && !retention).then_some(routed_full).flatten(),
            reference_estimate_micros: complete.then_some(baseline).flatten(),
            estimated_savings_micros: difference,
            same_usage_bounds,
            compared_routed_subtotal_micros: (compared>0).then_some(actual).flatten(),
            compared_reference_subtotal_micros: (compared>0).then_some(baseline).flatten(),
            actual_cash_micros: None,
            disclosure: concat!("API-equivalent estimate using current source-specific prices and recorded tokens, including retries. ",
                "The reference values accepted usage at the selected model's rates; equal output or quality is not established. ",
                "This does not reprice historical receipts or prove a bill or cash saving. ",
                "The full-scope routed amount can be available independently of reference coverage. Bounds assume the same input and cache-read usage and an unknown exclusive cache-write split between zero and input minus cache reads. ",
                "Missing/expired evidence is not zero; subtotals and comparison coverage count paired complete requests only.").into(),
        }
}

struct Comparison {
    complete: bool,
    actual: Option<u64>,
    accepted: Option<(ExecutionPricingEvidenceV1, RawUsage)>,
}
impl Default for Comparison {
    fn default() -> Self {
        Self {
            complete: false,
            actual: Some(0),
            accepted: None,
        }
    }
}

fn model_key(model: &hiroute_domain::PriceModelIdentityV1) -> &str {
    match model {
        hiroute_domain::PriceModelIdentityV1::CatalogModel(id)
        | hiroute_domain::PriceModelIdentityV1::LocalModel(id) => id,
    }
}
