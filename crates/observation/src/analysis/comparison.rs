//! One source-matched, per-attempt integer comparison used by both the legacy
//! one-shot report and retained analysis. Quote validation stays at admission.
use super::projection::{PricingIdentity, model_key};
use crate::{
    query_v2::ObservationV2Error as Error,
    valuation::{
        MetricCoverage, RawUsage, ValuationMissingReason, amount_only, reference_bounds_semantics,
        reference_usage_semantics,
    },
};
use hiroute_domain::*;
use rusqlite::{Connection, OptionalExtension, ToSql};
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Default)]
pub(super) struct RequestCost {
    pub id: String,
    pub anchor: String,
    pub attempts: u64,
    pub complete: bool,
    pub known: Option<u64>,
    pub known_seen: bool,
    pub missing_price: u64,
    pub missing_usage: u64,
    pub pending: bool,
    pub reference: Option<u64>,
    pub reference_bounds: Option<(u64, u64)>,
}

pub(super) struct Comparison {
    pub requests: BTreeMap<String, RequestCost>,
    pub excluded: u64,
    pub used: BTreeSet<String>,
}

#[allow(clippy::too_many_arguments)] // Keep SQL scope, price authority and read deadline explicit.
pub(super) fn evaluate(
    db: &Connection,
    selected: &str,
    args: &[(&str, &dyn ToSql)],
    prices: &[ApiEquivalentPriceV1],
    reference: Option<&ApiEquivalentPriceV1>,
    currency: &str,
    reader: &ObservationReaderContext,
    now: i64,
) -> Result<Comparison, Error> {
    let index = price_index(prices, currency);
    let mut identities =
        HashMap::<String, Option<(PricingIdentity, Option<&ApiEquivalentPriceV1>)>>::new();
    let mut identity_stmt =
        db.prepare("SELECT body FROM observation_analysis_pricing_v1 WHERE key=?1")?;
    let sql=format!("{selected} SELECT a.request_id,a.anchor_id,a.terminal,a.partial,a.accepted_ordinal,a.traffic,
        i.ordinal,i.pricing_key,i.input_tokens,i.output_tokens,i.cache_read,i.cache_write,i.reasoning
        FROM selected a LEFT JOIN observation_analysis_attempts_v1 i ON i.workspace_id=a.workspace_id
          AND i.request_id=a.request_id AND i.revision=a.revision");
    let mut stmt = db.prepare(&sql)?;
    let mut rows = stmt.query(args)?;
    let mut result = Comparison {
        requests: BTreeMap::new(),
        excluded: 0,
        used: BTreeSet::new(),
    };
    // Aggregation is keyed by request, so sorting every attempt is unnecessary.
    // Keep exclusion deduplication independent of the database's visit order too.
    let mut excluded_requests = BTreeSet::new();
    let mut n = 0u64;
    let mut validated_prices = BTreeSet::new();
    let mut validated_reference = false;
    while let Some(r) = rows.next()? {
        n += 1;
        if n.is_multiple_of(512) {
            reader.check(now, false, false)?;
        }
        let id: String = r.get(0)?;
        if r.get::<_, String>(5)? != "normal" {
            if excluded_requests.insert(id) {
                result.excluded += 1;
            }
            continue;
        }
        let c = result.requests.entry(id.clone()).or_insert(RequestCost {
            id,
            anchor: r.get(1)?,
            complete: r.get::<_, bool>(2)? && !r.get::<_, bool>(3)?,
            known: Some(0),
            ..Default::default()
        });
        let ordinal: Option<u64> = r.get(6)?;
        c.attempts += u64::from(ordinal.is_some());
        let key: Option<String> = r.get(7)?;
        if let Some(key) = &key
            && !identities.contains_key(key)
        {
            let body: Option<String> = identity_stmt.query_row([key], |r| r.get(0)).optional()?;
            let found = body
                .and_then(|v| serde_json::from_str::<PricingIdentity>(&v).ok())
                .map(|id| {
                    let price = if id.standard_tokens {
                        index
                            .get(&(
                                id.source_id.as_str(),
                                id.source_digest.as_str(),
                                id.model_id.as_str(),
                            ))
                            .copied()
                    } else {
                        None
                    };
                    (id, price)
                });
            identities.insert(key.clone(), found);
        }
        let identity = key
            .as_ref()
            .and_then(|key| identities.get(key))
            .and_then(Option::as_ref);
        let number = |column| -> rusqlite::Result<Option<u64>> {
            Ok(r.get::<_, Option<String>>(column)?
                .and_then(|s| s.parse().ok()))
        };
        let raw = RawUsage {
            input: number(8)?,
            output: number(9)?,
            read: number(10)?,
            write: number(11)?,
            reasoning: number(12)?,
        };
        if let Some((identity, Some(price))) = identity {
            if validated_prices.insert(price.binding_id.as_str()) {
                crate::valuation::validate_quote(&price.quote).map_err(|_| Error::Invalid)?;
            }
            result.used.insert(price.binding_id.clone());
            let amount = amount_only(
                &price.quote,
                &reference_usage_semantics(identity.semantics, &raw, &price.quote),
            );
            if amount.coverage != MetricCoverage::Complete {
                c.complete = false;
                c.missing_usage += u64::from(
                    amount
                        .missing
                        .contains(&Some(ValuationMissingReason::UsageUnknown)),
                );
                c.missing_price += u64::from(
                    amount
                        .missing
                        .contains(&Some(ValuationMissingReason::PriceUnknown)),
                );
            }
            if let Some(v) = amount.known_sum_micros {
                c.known_seen = true;
                c.known = c.known.and_then(|s| s.checked_add(v));
            } else {
                c.complete = false;
            }
        } else {
            c.complete = false;
            c.missing_price += 1;
        }
        let accepted: Option<u64> = r.get(4)?;
        if ordinal.is_some() && ordinal == accepted {
            if let Some(p) = reference.filter(|_| identity.is_some() && !validated_reference) {
                crate::valuation::validate_quote(&p.quote).map_err(|_| Error::Invalid)?;
                validated_reference = true;
            }
            c.reference = reference.zip(identity).and_then(|(p, (i, _))| {
                let a = amount_only(
                    &p.quote,
                    &reference_usage_semantics(i.semantics, &raw, &p.quote),
                );
                (a.coverage == MetricCoverage::Complete)
                    .then_some(a.known_sum_micros)
                    .flatten()
            });
            c.reference_bounds = c.reference.map(|amount| (amount, amount)).or_else(|| {
                reference
                    .zip(identity)
                    .and_then(|(p, (i, _))| reference_bounds_semantics(i.semantics, &raw, &p.quote))
            });
        }
    }
    Ok(result)
}

fn price_index<'a>(
    prices: &'a [ApiEquivalentPriceV1],
    currency: &str,
) -> HashMap<(&'a str, &'a str, &'a str), &'a ApiEquivalentPriceV1> {
    let mut map = HashMap::new();
    for p in prices {
        if p.quote.currency != currency {
            continue;
        }
        let target = &p.quote.exact_target;
        map.entry((
            target.source_id.as_str(),
            target.source_identity_digest.as_str(),
            model_key(&target.model_identity),
        ))
        .or_insert(p);
        if let Some(id) = &p.execution_model_configuration_id {
            map.entry((
                target.source_id.as_str(),
                target.source_identity_digest.as_str(),
                id.as_str(),
            ))
            .or_insert(p);
        }
    }
    map
}
