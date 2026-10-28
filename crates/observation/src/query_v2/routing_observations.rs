//! All matching retained decision events, independent of stage pagination and grade filters.
use super::{ObservationReaderContext, ObservationV2Error};
use hiroute_domain::*;
use rusqlite::{Transaction, named_params};
use std::collections::{BTreeMap, BTreeSet};

const MAX_DECISIONS: usize = 10_000;
const MAX_SAFE_BYTES: usize = 32 * 1024 * 1024;

// The partial index excludes unrelated safe projections before opening JSON
// payload pages. Invalid legacy JSON remains visible to the original predicate
// so the read still fails closed, while index creation can preserve those bytes.
// Covered safe bodies and producer identities avoid scattered table reads on a
// cold store. The fixed join order bounds retained producer-identity lookups.
// Stream in the covering index's unique request/digest order: ordering by rowid
// would sort every match before LIMIT or the reader's byte ceiling can stop it.
// This is an internal scan order; stage pages keep their existing time order.
pub(super) const SUMMARY_SQL: &str = "SELECT f.body_json,e.rowid
        FROM observation_safe_facts_v2 f INDEXED BY observation_safe_route_payloads
        CROSS JOIN execution_fact_events e INDEXED BY execution_facts_request_payload ON e.workspace_id=f.workspace_id AND e.request_id=f.request_id AND e.envelope_digest=f.original_digest
        JOIN logical_requests r INDEXED BY requests_observation_fields ON r.workspace_id=f.workspace_id AND r.request_id=f.request_id
        JOIN sessions s ON s.workspace_id=r.workspace_id AND s.session_id=r.session_id
        LEFT JOIN observation_run_links l ON l.workspace_id=r.workspace_id AND l.request_id=r.request_id
        WHERE f.workspace_id=:workspace
          AND CASE WHEN json_valid(f.body_json) THEN json_extract(f.body_json,'$.event_kind') ELSE 'route_decision' END='route_decision'
          AND json_extract(f.body_json,'$.event_kind')='route_decision'
          AND COALESCE(json_extract(f.body_json,'$.occurred_at_ms'),r.started_at_ms)>=:from AND COALESCE(json_extract(f.body_json,'$.occurred_at_ms'),r.started_at_ms)<:to
          AND (:session IS NULL OR r.session_id=:session)
          AND (:plan IS NULL OR COALESCE(json_extract(f.body_json,'$.routing_observation.plan_id'),json_extract(f.body_json,'$.routing_context.plan_id'))=:plan
               OR COALESCE(json_extract(f.body_json,'$.routing_observation.plan_id'),json_extract(f.body_json,'$.routing_context.plan_id')) IS NULL)
          AND (:revision IS NULL OR CAST(COALESCE(json_extract(f.body_json,'$.routing_observation.plan_revision'),json_extract(f.body_json,'$.routing_context.plan_revision')) AS INTEGER)=:revision
               OR COALESCE(json_extract(f.body_json,'$.routing_observation.plan_revision'),json_extract(f.body_json,'$.routing_context.plan_revision')) IS NULL)
          AND (:runs IS NULL OR (l.conflicted=0 AND l.run_id IN(SELECT value FROM json_each(:runs))))
          AND (s.tombstone_reason IS NULL OR EXISTS(SELECT 1 FROM observation_tombstones t WHERE t.workspace_id=r.workspace_id AND t.session_id=r.session_id AND t.delete_scope='content_only'))
        ORDER BY f.request_id,f.original_digest LIMIT 10001";

pub(super) fn summarize(
    transaction: &Transaction<'_>,
    reader: &ObservationReaderContext,
    query: &PlanQualitySamplesQuery,
    from_ms: i64,
    to_ms: i64,
    runs: &Option<String>,
) -> Result<RoutingObservationsSummaryV1, ObservationV2Error> {
    let mut result = RoutingObservationsSummaryV1 {
        schema: "hiroute.routing-observations/v1".into(),
        plan_id: query.plan_id.clone(),
        session_id: query.session_id.clone(),
        plan_revision: query.plan_revision,
        requested_from_ms: query.from_ms,
        requested_to_ms: query.to_ms,
        covered_from_ms: from_ms,
        covered_to_ms: to_ms,
        coverage: "complete".into(),
        retained_decisions: 0,
        known_non_c_decisions: 0,
        legacy_or_unknown_decisions: 0,
        unknown_scope_decisions: 0,
        missing_projection_events: 0,
        scan_limited: false,
        cohorts: vec![],
    };
    let mut cohorts: BTreeMap<RoutingObservationCohortV1, RoutingObservationCountsV1> =
        BTreeMap::new();
    let mut seen = BTreeSet::new();
    let mut statement = transaction.prepare(SUMMARY_SQL)?;
    let mut rows = statement.query(named_params! { ":workspace":reader.workspace().as_str(),":from":from_ms,":to":to_ms,":session":query.session_id,":plan":query.plan_id,":revision":query.plan_revision,":runs":runs })?;
    let mut bytes = 0;
    let mut scanned = 0;
    while let Some(row) = rows.next()? {
        let body: String = row.get(0)?;
        bytes += body.len();
        scanned += 1;
        if scanned > MAX_DECISIONS || bytes > MAX_SAFE_BYTES {
            result.scan_limited = true;
            break;
        }
        // The UNIQUE producer/epoch/stream/event key makes rowid an equivalent
        // identity within this snapshot; the narrow request index covers it.
        let identity = row.get::<_, i64>(1)?;
        if !seen.insert(identity) {
            continue;
        }
        let fact: ObservationSafeFactV2 =
            serde_json::from_str(&body).map_err(|_| ObservationV2Error::Unavailable)?;
        let plan_id = fact
            .routing_observation
            .as_ref()
            .map(|o| o.plan_id.as_str())
            .or_else(|| {
                fact.routing_context
                    .as_ref()
                    .and_then(|c| c.plan_id.as_deref())
            });
        let revision = fact
            .routing_observation
            .as_ref()
            .map(|o| o.plan_revision)
            .or_else(|| {
                fact.routing_context
                    .as_ref()
                    .and_then(|c| c.plan_revision.as_ref().and_then(|r| r.parse().ok()))
            });
        if (query.plan_id.is_some() && plan_id.is_none())
            || (query.plan_revision.is_some() && revision.is_none())
        {
            result.unknown_scope_decisions += 1;
            continue;
        }
        result.retained_decisions += 1;
        if fact.occurred_at_ms.is_none() {
            result.legacy_or_unknown_decisions += 1;
            continue;
        }
        match fact.decision_protocol {
            Some(DecisionProtocolV1::SystemOneC) => {}
            Some(_) => {
                result.known_non_c_decisions += 1;
                continue;
            }
            None if fact.routing_observation.is_none()
                && fact
                    .pre_applicability_schema_digest
                    .as_deref()
                    .is_some_and(is_pre_applicability_execution_digest) =>
            {
                result.known_non_c_decisions += 1;
                continue;
            }
            None => {
                result.legacy_or_unknown_decisions += 1;
                continue;
            }
        }
        let Some(record) = fact.routing_observation else {
            result.legacy_or_unknown_decisions += 1;
            continue;
        };
        let o = &record.observation;
        if !o.validate() {
            result.legacy_or_unknown_decisions += 1;
            continue;
        }
        let cohort = RoutingObservationCohortV1 {
            protocol: o.protocol.clone(),
            content_digest: o.content_digest.clone(),
            bundle_digest: o.bundle_digest.clone(),
            scope_id: record.selected_branch_id,
            standard: o.standard.clone(),
            provider: o.provider.clone(),
            model: o.model.clone(),
            simple_threshold_millis: o.simple_threshold_millis,
            competence_floor_millis: o.competence_floor_millis,
        };
        let counts = cohorts.entry(cohort).or_default();
        if record.inherited {
            counts.inherited += 1;
            continue;
        }
        counts.c_decisions += 1;
        if o.applicability_requested {
            counts.requested += 1;
            if o.applicability_state == ApplicabilityStateV1::Unavailable {
                counts.unavailable += 1;
            }
        }
        match o.applicability_reason {
            ApplicabilityReasonV1::NoTarget => counts.no_target_omissions += 1,
            ApplicabilityReasonV1::PartialTarget => counts.partial_target_omissions += 1,
            ApplicabilityReasonV1::RequiredQuestionMissing => counts.required_question_missing += 1,
            _ => {}
        }
        let opportunity = o.eligibility == ProtectionEligibilityV1::Eligible
            && o.score
                .as_ref()
                .and_then(serde_json::Number::as_f64)
                .is_some_and(|s| s < f64::from(o.competence_floor_millis) / 1000.0)
            && matches!(
                o.protection_action,
                HistoricalProtectionActionV1::Applied
                    | HistoricalProtectionActionV1::RetainedUnavailable
                    | HistoricalProtectionActionV1::Suppressed
            );
        if opportunity {
            counts.gate_opportunities += 1;
        }
        if opportunity && o.protection_action == HistoricalProtectionActionV1::Suppressed {
            counts.suppressed += 1;
        }
    }
    // Missing safe projections are observable gaps. With no safe Plan identity their Plan
    // membership is uncertain, so conservatively withhold exact-population rates.
    result.missing_projection_events = transaction.query_row("SELECT COUNT(*) FROM execution_fact_events e INDEXED BY execution_facts_request_payload JOIN logical_requests r INDEXED BY requests_observation_fields ON r.workspace_id=e.workspace_id AND r.request_id=e.request_id
        LEFT JOIN observation_run_links l ON l.workspace_id=r.workspace_id AND l.request_id=r.request_id
        WHERE e.workspace_id=:workspace AND r.started_at_ms>=:from AND r.started_at_ms<:to AND (:session IS NULL OR r.session_id=:session)
          AND (:runs IS NULL OR (l.conflicted=0 AND l.run_id IN(SELECT value FROM json_each(:runs))))
          AND NOT EXISTS(SELECT 1 FROM observation_safe_facts_v2 f INDEXED BY observation_safe_kind_fields WHERE f.workspace_id=e.workspace_id AND f.request_id=e.request_id AND f.original_digest=e.envelope_digest)",
        named_params! {":workspace":reader.workspace().as_str(),":from":from_ms,":to":to_ms,":session":query.session_id,":runs":runs}, |r| r.get(0))?;
    let complete = !result.scan_limited
        && result.missing_projection_events == 0
        && result.legacy_or_unknown_decisions == 0
        && result.unknown_scope_decisions == 0;
    if !complete {
        result.coverage = "partial".into();
    }
    let rate = |n, d| {
        if complete && d > 0 {
            Some(n as f64 / d as f64)
        } else {
            None
        }
    };
    result.cohorts = cohorts
        .into_iter()
        .map(|(cohort, mut counts)| {
            counts.unavailable_per_requested = rate(counts.unavailable, counts.requested);
            counts.suppressed_per_opportunity = rate(counts.suppressed, counts.gate_opportunities);
            counts.suppressed_per_c_decision = rate(counts.suppressed, counts.c_decisions);
            RoutingObservationCohortSummaryV1 { cohort, counts }
        })
        .collect();
    Ok(result)
}
