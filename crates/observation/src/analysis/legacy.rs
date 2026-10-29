//! Legacy one-shot comparison keeps its original time and physical-session
//! meaning while sharing ready numeric inputs. Old/unready auxiliary formats
//! use the existing bounded reader path, never a row-count admission limit.
use super::{comparison, schema};
use crate::query_v2::ObservationV2Error as Error;
use hiroute_domain::*;
use rusqlite::{Connection, named_params};
use std::collections::BTreeSet;

pub(crate) struct ProjectedComparison {
    pub total: u64,
    pub compared: u64,
    /// Full-scope routed estimate, independent of reference-price coverage.
    pub routed_full: Option<u64>,
    pub routed_overflow: bool,
    /// Full-scope reference bounds under the explicit same-usage scenario.
    pub reference_bounds: Option<(u64, u64)>,
    pub actual: Option<u64>,
    pub baseline: Option<u64>,
    pub used: BTreeSet<String>,
    pub excluded: u64,
}

pub(crate) fn projected_comparison(
    db: &Connection,
    reader: &ObservationReaderContext,
    q: &ObservationValueQueryV2,
    reference: &ApiEquivalentPriceV1,
    prices: &[ApiEquivalentPriceV1],
    from: i64,
    now: i64,
) -> Result<Option<ProjectedComparison>, Error> {
    if !schema::available(db) {
        return Ok(None);
    }
    let filter="FROM logical_requests r JOIN sessions s ON s.workspace_id=r.workspace_id AND s.session_id=r.session_id
      LEFT JOIN valuation_requests_v2 v ON v.workspace_id=r.workspace_id AND v.request_id=r.request_id
      WHERE r.workspace_id=:workspace AND r.started_at_ms>=:from AND r.started_at_ms<:to
        AND (:session IS NULL OR r.session_id=:session) AND (:plan IS NULL OR v.plan_id=:plan)
        AND (s.tombstone_reason IS NULL OR s.content_completeness='deleted')";
    let args = named_params! {":workspace":reader.workspace().as_str(),":from":from,":to":q.to_ms,":session":q.session_id,":plan":q.plan_id};
    let pending:bool=db.query_row(&format!("SELECT EXISTS(SELECT 1 {filter}
        AND (EXISTS(SELECT 1 FROM observation_analysis_dirty_v1 d WHERE d.workspace_id=r.workspace_id AND d.request_id=r.request_id)
        OR NOT EXISTS(SELECT 1 FROM observation_analysis_requests_v1 a WHERE a.workspace_id=r.workspace_id AND a.request_id=r.request_id)))"),args,|r|r.get(0))?;
    if pending {
        return Ok(None);
    }
    let cte="WITH selected AS (SELECT a.*,r.session_id AS anchor_id
      FROM logical_requests r CROSS JOIN observation_analysis_requests_v1 a
      JOIN sessions s ON s.workspace_id=r.workspace_id AND s.session_id=r.session_id
      LEFT JOIN valuation_requests_v2 v ON v.workspace_id=r.workspace_id AND v.request_id=r.request_id
      WHERE r.workspace_id=:workspace AND r.started_at_ms>=:from AND r.started_at_ms<:to
        AND (:session IS NULL OR r.session_id=:session) AND (:plan IS NULL OR v.plan_id=:plan)
        AND (s.tombstone_reason IS NULL OR s.content_completeness='deleted')
        AND a.workspace_id=r.workspace_id AND a.request_id=r.request_id
        AND a.revision=(SELECT MAX(revision) FROM observation_analysis_requests_v1 latest
            WHERE latest.workspace_id=r.workspace_id AND latest.request_id=r.request_id))";
    let data = comparison::evaluate(
        db,
        cte,
        args,
        prices,
        Some(reference),
        q.currency.as_deref().ok_or(Error::Invalid)?,
        reader,
        now,
    )?;
    Ok(Some(aggregate(data)))
}

pub(super) fn aggregate(data: comparison::Comparison) -> ProjectedComparison {
    let mut result = ProjectedComparison {
        total: data.requests.len() as u64,
        compared: 0,
        routed_full: Some(0),
        routed_overflow: false,
        reference_bounds: Some((0, 0)),
        actual: Some(0),
        baseline: Some(0),
        used: data.used,
        excluded: data.excluded,
    };
    for c in data.requests.into_values() {
        if c.complete && c.known_seen {
            if let Some(known) = c.known {
                if let Some(sum) = result.routed_full {
                    result.routed_full = sum.checked_add(known);
                    result.routed_overflow |= result.routed_full.is_none();
                }
            } else {
                result.routed_full = None;
                result.routed_overflow = true;
            }
        } else {
            result.routed_full = None;
        }
        if c.complete && c.known_seen && c.known.is_some() {
            result.reference_bounds = result.reference_bounds.zip(c.reference_bounds).and_then(
                |((lower, upper), (request_lower, request_upper))| {
                    Some((
                        lower.checked_add(request_lower)?,
                        upper.checked_add(request_upper)?,
                    ))
                },
            );
        } else {
            result.reference_bounds = None;
        }
        if c.complete && c.reference.is_some() && c.known.is_some() {
            result.compared += 1;
            result.actual = result.actual.and_then(|s| s.checked_add(c.known?));
            result.baseline = result.baseline.and_then(|s| s.checked_add(c.reference?));
        }
    }
    result
}
