//! Select one exact trial scope; all token pricing and rounding stay in comparison::evaluate.
use super::{
    comparison,
    legacy::{ProjectedComparison, aggregate},
    schema,
};
use crate::query_v2::ObservationV2Error as Error;
use hiroute_domain::*;
use rusqlite::{Connection, named_params};

// Lack of a turn-closing fact is not proof that an Agent has stopped. After this
// interval without retained session activity it is a cost-evidence gap, not an
// indefinitely pending projection. Late facts are read afresh on every query.
const UNRESOLVED_AFTER_MS: i64 = 5 * 60 * 1000;

const SCOPE: &str = "WITH candidates AS (
    SELECT r.workspace_id,r.request_id,r.session_id,s.updated_at_ms,t.segment_id,:idle AS idle_before,
      COALESCE(t.membership_conflict,0) AS conflict,q.plan_id AS stage_plan,
      q.plan_revision AS stage_revision,
      COALESCE(q.executed_branch_id,q.selected_branch_id) AS stage_branch,
      json_extract(q.branch_execution_json,'$.group') AS stage_group,
      a.request_id AS projected,
      EXISTS(SELECT 1 FROM observation_analysis_dirty_v1 d
        WHERE d.workspace_id=r.workspace_id AND d.request_id=r.request_id) AS dirty
    FROM logical_requests r
    JOIN sessions s ON s.workspace_id=r.workspace_id AND s.session_id=r.session_id
    JOIN valuation_requests_v2 v ON v.workspace_id=r.workspace_id AND v.request_id=r.request_id
    LEFT JOIN observation_analysis_turns_v1 t ON t.workspace_id=r.workspace_id AND t.request_id=r.request_id
    LEFT JOIN plan_quality_segments q ON q.workspace_id=r.workspace_id AND q.segment_id=t.segment_id AND q.session_id=r.session_id
    LEFT JOIN observation_analysis_requests_v1 a ON a.workspace_id=r.workspace_id AND a.request_id=r.request_id
      AND a.revision=(SELECT MAX(latest.revision) FROM observation_analysis_requests_v1 latest
        WHERE latest.workspace_id=r.workspace_id AND latest.request_id=r.request_id)
    WHERE r.workspace_id=:workspace AND v.plan_id=:plan AND r.started_at_ms>=:from AND r.started_at_ms<:to
      AND r.traffic_kind='normal' AND (s.tombstone_reason IS NULL OR s.content_completeness='deleted')
  ), scoped AS (
    SELECT *, COALESCE(conflict=0 AND stage_plan=:plan AND (
      (:revision IS NOT NULL AND stage_revision!=:revision)
      OR (:branch IS NOT NULL AND stage_branch!=:branch)
      OR (stage_group IN ('regular','primary') AND stage_group!=:group)),0) AS outside,
      COALESCE(conflict=0 AND stage_plan=:plan AND stage_revision IS NOT NULL
        AND (:revision IS NULL OR stage_revision=:revision)
        AND (:branch IS NULL OR stage_branch=:branch) AND stage_group=:group,0) AS matched
    FROM candidates
  )";

pub(crate) fn trust_comparison(
    db: &Connection,
    reader: &ObservationReaderContext,
    scope: &TrustCostScopeV1,
    reference: &ApiEquivalentPriceV1,
    prices: &[ApiEquivalentPriceV1],
    retained: i64,
    now: i64,
) -> Result<ProjectedComparison, Error> {
    if !schema::available(db) {
        return Err(Error::Unavailable);
    }
    let group = match scope.group {
        ExecutionGroupV1::Regular => "regular",
        ExecutionGroupV1::Primary => "primary",
    };
    let args = named_params! {":workspace":reader.workspace().as_str(),":plan":scope.plan_id,":revision":scope.revision,":branch":scope.branch_id,":group":group,":from":scope.from_ms.max(retained),":to":scope.to_ms,":idle":now.saturating_sub(UNRESOLVED_AFTER_MS)};
    // Unknown membership stays in the conservative trial denominator. Only an
    // actual catch-up or a recent unclosed turn holds the entire result pending.
    let (pending, gaps): (bool, u64) = db.query_row(
        &format!(
            "{SCOPE} SELECT COALESCE(MAX(dirty OR projected IS NULL
            OR (segment_id IS NULL AND updated_at_ms>idle_before)),0),
            COALESCE(SUM(NOT matched),0) FROM scoped WHERE NOT outside"
        ),
        args,
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if pending {
        return Err(Error::Unavailable);
    }
    let selected = format!("{SCOPE}, selected AS (SELECT a.*,c.session_id AS anchor_id FROM scoped c
        JOIN observation_analysis_requests_v1 a ON a.workspace_id=c.workspace_id AND a.request_id=c.request_id
        WHERE c.matched AND a.revision=(SELECT MAX(latest.revision) FROM observation_analysis_requests_v1 latest
          WHERE latest.workspace_id=c.workspace_id AND latest.request_id=c.request_id))");
    let data = comparison::evaluate(
        db,
        &selected,
        args,
        prices,
        Some(reference),
        &scope.currency,
        reader,
        now,
    )?;
    let mut result = aggregate(data);
    // Unlike the generic report's non-normal traffic exclusions, these gaps are
    // also missing requests. Adding them to total prevents a false full-scope
    // difference without changing the shared report or numeric engine.
    result.total += gaps;
    result.excluded += gaps;
    Ok(result)
}
