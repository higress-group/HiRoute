use hiroute_domain::{
    AgentTurnAttributionV1, CanonicalDigest, PlanQualityAssessment, PlanQualityExecutionIdentity,
    PlanQualityModelSummary, PlanQualitySample, PlanQualitySamplesPage, PlanQualitySamplesQuery,
    PlanQualitySummary,
};
use rusqlite::{Connection, OpenFlags, Transaction, named_params};
use serde::{Deserialize, Serialize};

use super::{ObservationReaderContext, ObservationV2Error, QueryDeadline, identifier};
use crate::LocalObservationStore;

#[cfg(test)]
#[path = "plan_quality_snapshot_tests.rs"]
mod snapshot_tests;

/// Both projections share the same scope and transaction. Detail filters never
/// enter this CTE, so pagination and drill-down cannot change the full-scope mean.
/// The request-scoped candidate index excludes unrelated content carriers.
/// The safe-event guard preserves missing projections and selected corruption.
/// Guarded CASE expressions match the small JSON expression indexes. The inner
/// CASE is the indexed value; the outer invalid-JSON branch preserves the original
/// read failure instead of treating corrupt content as a missing field.
/// A request tombstone with no current membership retires facts before physical
/// cleanup. Content-only deletion keeps that membership and its quality evidence.
pub(super) const SCOPE: &str = "
    SELECT q.*,
      json_extract(q.branch_execution_json,'$.group') AS execution_group,
      json_extract(q.branch_execution_json,'$.candidate_index') AS candidate_index,
      json_extract(q.branch_execution_json,'$.policy') AS branch_policy,
      json_extract(q.branch_execution_json,'$.policy.floor_millis') AS floor_millis,
      retired_assessment.request_id IS NOT NULL AS assessment_retired,
      CASE WHEN q.assessment_partial=0 AND retired_assessment.request_id IS NULL
        AND COALESCE(json_extract(q.history_membership_json,'$.clipped'),0)=0
        AND COALESCE(json_extract(q.history_membership_json,'$.capture_partial'),0)=0
        AND (q.assessment_membership_json IS NULL OR NOT EXISTS(SELECT 1 FROM json_each(q.history_membership_json,'$.request_ids') actual WHERE NOT EXISTS(SELECT 1 FROM json_each(q.assessment_membership_json,'$.request_ids') assessed WHERE assessed.value=actual.value))) THEN q.score END AS reliable_score,
      (COALESCE(json_extract(q.history_membership_json,'$.clipped'),0)=0
        AND COALESCE(json_extract(q.history_membership_json,'$.capture_partial'),0)=0
        AND (q.assessment_membership_json IS NULL OR NOT EXISTS(SELECT 1 FROM json_each(q.history_membership_json,'$.request_ids') actual WHERE NOT EXISTS(SELECT 1 FROM json_each(q.assessment_membership_json,'$.request_ids') assessed WHERE assessed.value=actual.value)))) AS assessment_covers_stage,
      COALESCE(
        (SELECT a.model_id FROM observation_attempt_models_v2 a INDEXED BY observation_attempt_model_names
         JOIN valuation_requests_v2 v INDEXED BY valuation_request_accepted_ordinal ON v.workspace_id=a.workspace_id
           AND v.request_id=a.request_id AND v.accepted_ordinal=a.ordinal
         JOIN logical_requests r INDEXED BY requests_observation_fields ON r.workspace_id=a.workspace_id AND r.request_id=a.request_id
         WHERE a.workspace_id=q.workspace_id AND a.request_id=q.last_request_id
           AND r.session_id=q.session_id LIMIT 1),
        (SELECT a.model_id FROM observation_attempt_models_v2 a INDEXED BY observation_attempt_model_names
         JOIN valuation_requests_v2 v INDEXED BY valuation_request_accepted_ordinal ON v.workspace_id=a.workspace_id
           AND v.request_id=a.request_id AND v.accepted_ordinal=a.ordinal
         JOIN logical_requests r INDEXED BY requests_observation_fields ON r.workspace_id=a.workspace_id AND r.request_id=a.request_id
         WHERE a.workspace_id=q.workspace_id AND a.request_id=q.first_request_id
           AND r.session_id=q.session_id LIMIT 1)
      ) AS native_model,
      (SELECT (CASE WHEN json_valid(p.body_json) THEN CASE WHEN json_valid(p.body_json) THEN json_extract(p.body_json,'$.fact.reasoning_profile_id') END ELSE json_extract(p.body_json,'$.fact.reasoning_profile_id') END)
       FROM observation_sensitive_payloads_v2 p INDEXED BY observation_payload_candidate_request
       CROSS JOIN execution_fact_events e INDEXED BY execution_facts_request_payload ON e.workspace_id=p.workspace_id AND e.request_id=p.request_id AND e.envelope_digest=substr(p.id,6)
       LEFT JOIN observation_safe_facts_v2 f INDEXED BY observation_safe_kind_fields ON f.workspace_id=e.workspace_id AND f.request_id=e.request_id AND f.original_digest=e.envelope_digest
       JOIN logical_requests r INDEXED BY requests_observation_fields ON r.workspace_id=e.workspace_id AND r.request_id=e.request_id
       WHERE p.workspace_id=q.workspace_id AND p.request_id IN(q.last_request_id,q.first_request_id)
         AND p.id GLOB 'fact:*' AND CASE WHEN json_valid(p.body_json) THEN json_extract(p.body_json,'$.fact.kind') ELSE 'candidate_decision' END='candidate_decision'
         AND e.workspace_id=q.workspace_id AND r.session_id=q.session_id
         AND e.request_id IN(q.last_request_id,q.first_request_id)
         AND CASE WHEN (f.original_digest IS NULL OR (CASE WHEN json_valid(f.body_json) THEN CASE WHEN json_valid(f.body_json) THEN json_extract(f.body_json,'$.event_kind') END ELSE json_extract(f.body_json,'$.event_kind') END)='candidate_decision') THEN
           (CASE WHEN json_valid(p.body_json) THEN CASE WHEN json_valid(p.body_json) THEN json_extract(p.body_json,'$.fact.kind') END ELSE json_extract(p.body_json,'$.fact.kind') END)='candidate_decision'
           AND (CASE WHEN json_valid(p.body_json) THEN CASE WHEN json_valid(p.body_json) THEN json_extract(p.body_json,'$.fact.model_configuration_id') END ELSE json_extract(p.body_json,'$.fact.model_configuration_id') END)=q.model_configuration_id
           AND (CASE WHEN json_valid(p.body_json) THEN CASE WHEN json_valid(p.body_json) THEN json_extract(p.body_json,'$.fact.profile_digest') END ELSE json_extract(p.body_json,'$.fact.profile_digest') END)=q.profile_digest
           ELSE 0 END
       ORDER BY e.sequence DESC LIMIT 1
      ) AS reasoning_profile_id
    FROM plan_quality_segments q
    JOIN sessions s ON s.workspace_id=q.workspace_id AND s.session_id=q.session_id
    LEFT JOIN observation_request_tombstones_v2 retired_assessment
      ON retired_assessment.workspace_id=q.workspace_id
      AND retired_assessment.request_id=q.assessment_trigger_request_id
      AND NOT EXISTS(SELECT 1 FROM logical_requests retained
        WHERE retained.workspace_id=q.workspace_id
          AND retained.request_id=q.assessment_trigger_request_id)
    LEFT JOIN observation_run_links l ON l.workspace_id=q.workspace_id
      AND l.request_id=q.first_request_id
    WHERE q.workspace_id=:workspace AND q.last_at_ms>=:from AND q.last_at_ms<:to
      AND q.last_at_ms IS NOT NULL AND q.selected_branch_id IS NOT NULL
      AND NOT EXISTS(SELECT 1 FROM observation_request_tombstones_v2 retired
        WHERE retired.workspace_id=q.workspace_id AND retired.request_id=q.last_request_id
          AND NOT EXISTS(SELECT 1 FROM logical_requests retained
            WHERE retained.workspace_id=q.workspace_id AND retained.request_id=q.last_request_id))
      AND (:plan IS NULL OR q.plan_id=:plan)
      AND (:session IS NULL OR q.session_id=:session)
      AND (:revision IS NULL OR q.plan_revision=:revision)
      AND (:runs IS NULL OR (l.conflicted=0 AND
        l.run_id IN(SELECT value FROM json_each(:runs))))
      AND (s.tombstone_reason IS NULL OR EXISTS(
        SELECT 1 FROM observation_tombstones t
        WHERE t.workspace_id=q.workspace_id AND t.session_id=q.session_id
          AND t.delete_scope='content_only'))
";

// Event IDs belong to producer streams; the trigger request also binds the
// assessment lookup and lets it use the existing request index. Selection uses
// the covering identity index and safe kind before opening its content carrier;
// an ordered non-covering event scan otherwise reads unrelated payload pages.
pub(super) const SAMPLE_SELECT: &str = "SELECT q.segment_id,q.session_id,q.plan_id,q.plan_revision,
                        q.selected_branch_id,q.executed_branch_id,q.model_configuration_id,
                        q.profile_digest,q.attribution,q.first_turn_id,q.first_turn_ordinal,
                        q.last_observed_turn_id,q.last_observed_turn_ordinal,q.first_at_ms,
                        q.last_at_ms,q.history_partial,q.first_request_id,q.last_request_id,
                        EXISTS(SELECT 1 FROM logical_requests first_req INDEXED BY requests_observation_fields
                          WHERE first_req.workspace_id=q.workspace_id
                            AND first_req.session_id=q.session_id
                            AND first_req.request_id=q.first_request_id)
                          AND EXISTS(SELECT 1 FROM logical_requests last_req INDEXED BY requests_observation_fields
                          WHERE last_req.workspace_id=q.workspace_id
                            AND last_req.session_id=q.session_id
                            AND last_req.request_id=q.last_request_id),
                        CASE WHEN COALESCE(l.conflicted,0)=0 THEN json_extract(l.body_json,'$.task_id') END,
                        CASE WHEN COALESCE(l.conflicted,0)=0 THEN l.run_id END,
                        CASE WHEN NOT q.assessment_retired THEN q.assessment_event_id END,
                        q.assessment_trigger_request_id,q.assessed_at_ms,
                        q.target_from_turn_id,q.target_through_turn_id,q.target_from_ordinal,
                        q.target_through_ordinal,q.score,
                        (q.assessment_partial OR NOT q.assessment_covers_stage),
                        CASE WHEN q.reason_present=1 AND NOT q.assessment_retired
                          THEN
           (CASE WHEN json_valid(p.body_json) THEN CASE WHEN json_valid(p.body_json) THEN json_extract(p.body_json,'$.fact.reason') END ELSE json_extract(p.body_json,'$.fact.reason') END) END,
                        q.assessment_event_id IS NOT NULL AND EXISTS(
                          SELECT 1 FROM logical_requests trigger_req INDEXED BY requests_observation_fields
                          WHERE trigger_req.workspace_id=q.workspace_id
                            AND trigger_req.session_id=q.session_id
                            AND trigger_req.request_id=q.assessment_trigger_request_id),
                        q.native_model,q.reasoning_profile_id,q.branch_execution_json,q.upgrade_json,
                        (SELECT (CASE WHEN json_valid(pick.body_json) THEN CASE WHEN json_valid(pick.body_json) THEN json_extract(pick.body_json,'$.fact.complexity') END ELSE json_extract(pick.body_json,'$.fact.complexity') END)
                         FROM execution_fact_events chosen INDEXED BY execution_facts_request_payload
                         LEFT JOIN observation_safe_facts_v2 chosen_safe INDEXED BY observation_safe_kind_fields ON chosen_safe.workspace_id=chosen.workspace_id AND chosen_safe.request_id=chosen.request_id AND chosen_safe.original_digest=chosen.envelope_digest
                         CROSS JOIN observation_sensitive_payloads_v2 pick INDEXED BY observation_payload_query_fields ON pick.id='fact:'||chosen.envelope_digest
                         WHERE chosen.workspace_id=q.workspace_id AND chosen.request_id=q.first_request_id
                           AND (chosen_safe.original_digest IS NULL OR (CASE WHEN json_valid(chosen_safe.body_json) THEN CASE WHEN json_valid(chosen_safe.body_json) THEN json_extract(chosen_safe.body_json,'$.event_kind') END ELSE json_extract(chosen_safe.body_json,'$.event_kind') END)='route_decision')
                           AND (CASE WHEN json_valid(pick.body_json) THEN CASE WHEN json_valid(pick.body_json) THEN json_extract(pick.body_json,'$.fact.kind') END ELSE json_extract(pick.body_json,'$.fact.kind') END)='route_decision'
                         ORDER BY chosen.sequence DESC LIMIT 1)
                 FROM quality q
                 LEFT JOIN observation_run_links l ON l.workspace_id=q.workspace_id
                    AND l.request_id=q.first_request_id
                 LEFT JOIN execution_fact_events e ON e.workspace_id=q.workspace_id
                    AND e.request_id=q.assessment_trigger_request_id
                    AND e.event_id=q.assessment_event_id
                    AND NOT q.assessment_retired
                 LEFT JOIN observation_sensitive_payloads_v2 p INDEXED BY observation_payload_query_fields
                    ON p.id='fact:'||e.envelope_digest";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    schema: String,
    binding: String,
    generation: u64,
    last_at_ms: i64,
    last_segment_id: String,
}

impl LocalObservationStore {
    pub fn observed_plan_quality_samples(
        &self,
        reader: &ObservationReaderContext,
        query: &PlanQualitySamplesQuery,
        now_ms: i64,
    ) -> Result<PlanQualitySamplesPage, ObservationV2Error> {
        self.plan_quality_samples_inner(reader, query, now_ms, || {})
    }

    fn plan_quality_samples_inner(
        &self,
        reader: &ObservationReaderContext,
        query: &PlanQualitySamplesQuery,
        now_ms: i64,
        after_snapshot: impl FnOnce(),
    ) -> Result<PlanQualitySamplesPage, ObservationV2Error> {
        reader.check(now_ms, false, false)?;
        validate(query)?;
        let _permit = self.query_permit()?;
        let mut normalized = query.clone();
        normalized.cursor = None;
        let binding = CanonicalDigest::of(&(reader.binding()?, normalized))
            .map_err(|_| ObservationV2Error::Invalid)?
            .to_string();
        let mut connection =
            Connection::open_with_flags(&self.activity_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let _deadline = QueryDeadline::start_for(&connection, reader)?;
        let transaction = connection.transaction()?;
        let (generation, visibility) = generations(&transaction)?;
        let cursor = match &query.cursor {
            Some(encoded) => {
                let cursor: Cursor = self.decode_observation_cursor(encoded)?;
                if cursor.schema != "plan-quality/v1"
                    || cursor.binding != binding
                    || cursor.generation != generation
                {
                    return Err(ObservationV2Error::Stale);
                }
                cursor
            }
            None => Cursor {
                schema: "plan-quality/v1".into(),
                binding,
                generation,
                last_at_ms: i64::MAX,
                last_segment_id: String::new(),
            },
        };
        let retained_from = now_ms
            .saturating_sub(crate::managed_text::RETENTION_MS)
            .saturating_add(1)
            .max(0);
        let from_ms = query.from_ms.unwrap_or(retained_from).max(retained_from);
        let to_ms = query.to_ms.unwrap_or_else(|| now_ms.saturating_add(1));
        if from_ms >= to_ms {
            return Err(ObservationV2Error::Invalid);
        }
        let runs = reader
            .allowed_runs()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|_| ObservationV2Error::Invalid)?;
        // Read request-ordered routing evidence before scattered stage lookups.
        // Both views share this snapshot; the global scan also avoids reopening
        // the same small identity indexes in alternating access orders.
        let routing_observations = query
            .include_routing_observations
            .then(|| {
                super::routing_observations::summarize(
                    &transaction,
                    reader,
                    query,
                    from_ms,
                    to_ms,
                    &runs,
                )
            })
            .transpose()?;
        let summary = summarize(&transaction, reader, query, from_ms, to_ms, &runs)?;
        let execution = query
            .execution
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|_| ObservationV2Error::Invalid)?;
        let detail = SAMPLE_SELECT.replace("ORDER BY chosen.sequence DESC LIMIT 1)", "ORDER BY chosen.sequence DESC LIMIT 1) AS selection").replace("FROM quality q", ", CASE WHEN l.conflicted=0 THEN json_extract(l.body_json,'$.harness_id') WHEN l.request_id IS NULL THEN (SELECT s.agent_id FROM sessions s WHERE s.workspace_id=q.workspace_id AND s.session_id=q.session_id) END AS executor FROM quality q");
        let mut rows = {
            let sql = format!(
                "WITH quality AS ({SCOPE})
                 {detail}
                 WHERE (:segment IS NULL OR q.segment_id=:segment)
                   AND (:model IS NULL OR q.model_configuration_id=:model)
                   AND (:execution_group IS NULL OR q.execution_group=:execution_group)
                   AND (:selection_reason IS NULL OR json_extract(selection,'$.selection_reason')=:selection_reason)
                   AND (json_array_length(:segments)=0 OR q.segment_id IN (SELECT value FROM json_each(:segments)))
                   AND (:executor IS NULL OR executor=:executor)
                   AND (:trust_row IS NULL OR (
                     COALESCE(q.executed_branch_id,q.selected_branch_id)=json_extract(:trust_row,'$.branch_id')
                     AND q.execution_group IS json_extract(:trust_row,'$.group')
                     AND q.model_configuration_id IS json_extract(:trust_row,'$.model_configuration_id')
                     AND q.reasoning_profile_id IS json_extract(:trust_row,'$.reasoning_profile_id')
                     AND q.attribution=json_extract(:trust_row,'$.attribution')
                     AND (json_extract(:trust_row,'$.group_by_executor')=0 OR executor IS json_extract(:trust_row,'$.executor'))))
                   AND (:unrated=0 OR q.reliable_score IS NULL)
                   AND (:gt IS NULL OR q.reliable_score>:gt)
                   AND (:lt IS NULL OR q.reliable_score<:lt)
                   AND (:competence IS NULL OR
                        (:competence='below_floor' AND q.reliable_score<q.floor_millis/1000.0) OR
                        (:competence='meets_floor' AND q.reliable_score>=q.floor_millis/1000.0))
                   AND (:execution IS NULL OR (
                     q.plan_revision=json_extract(:execution,'$.plan_revision')
                     AND q.execution_group IS json_extract(:execution,'$.group')
                     AND q.candidate_index IS json_extract(:execution,'$.candidate_index')
                     AND q.executed_branch_id IS json_extract(:execution,'$.executed_branch_id')
                     AND (q.executed_branch_id IS NOT NULL OR
                       q.selected_branch_id=json_extract(:execution,'$.selected_branch_id'))
                     AND q.model_configuration_id IS json_extract(:execution,'$.model_configuration_id')
                     AND q.profile_digest IS json_extract(:execution,'$.profile_digest')
                     AND q.attribution=json_extract(:execution,'$.attribution')))
                   AND (q.last_at_ms<:last_at OR
                        (q.last_at_ms=:last_at AND q.segment_id>:last_segment))
                 ORDER BY q.last_at_ms DESC,q.segment_id LIMIT :limit"
            );
            let mut statement = transaction.prepare(&sql)?;
            statement
                .query_map(
                    named_params! {
                        ":workspace": reader.workspace().as_str(),
                        ":from": from_ms, ":to": to_ms,
                        ":plan": query.plan_id, ":session": query.session_id,
                        ":revision": query.plan_revision.and_then(|v| i64::try_from(v).ok()),
                        ":runs": runs, ":segment": query.segment_id,
                        ":execution_group": query.execution_group.map(|g| match g { hiroute_domain::ExecutionGroupV1::Regular => "regular", hiroute_domain::ExecutionGroupV1::Primary => "primary" }),
                        ":selection_reason": query.selection_reason.map(|r| serde_json::to_value(r).expect("enum").as_str().expect("enum string").to_owned()),
                        ":segments": serde_json::to_string(&query.segment_ids).map_err(|_| ObservationV2Error::Invalid)?,
                        ":executor": query.executor,
                        ":trust_row": query.trust_row.as_ref().map(serde_json::to_string).transpose().map_err(|_| ObservationV2Error::Invalid)?,
                        ":model": query.model_configuration_id, ":unrated": query.unrated_only,
                        ":gt": query.score_gt, ":lt": query.score_lt,
                        ":competence": query.competence.map(|v| match v { hiroute_domain::PlanCompetenceFilter::BelowFloor => "below_floor", hiroute_domain::PlanCompetenceFilter::MeetsFloor => "meets_floor" }),
                        ":execution": execution, ":last_at": cursor.last_at_ms,
                        ":last_segment": cursor.last_segment_id,
                        ":limit": u64::from(query.limit) + 1,
                    },
                    sample,
                )?
                .collect::<Result<Vec<_>, _>>()?
        };
        let more = rows.len() > usize::from(query.limit);
        rows.truncate(usize::from(query.limit));
        if !query.include_routing_observations {
            for row in &mut rows {
                if let Some(selection) = row.selection.as_mut() {
                    selection.routing_observation = None;
                    selection.decision_protocol = None;
                }
            }
        }
        let next_cursor = if more {
            let last = rows.last().ok_or(ObservationV2Error::Unavailable)?;
            Some(self.encode_observation_cursor(&Cursor {
                last_at_ms: last.last_at_ms,
                last_segment_id: last.segment_id.clone(),
                ..cursor
            })?)
        } else {
            None
        };
        transaction.commit()?;
        after_snapshot();
        // Fresh pages already have a consistent summary and detail snapshot.
        // Ordinary ingestion must not invalidate that complete read. Visibility
        // changes still fence every response; pagination keeps its strict
        // generation contract so it cannot mix populations across pages.
        let (current, current_visibility) = generations(&connection)?;
        if current_visibility != visibility || (query.cursor.is_some() && current != generation) {
            return Err(ObservationV2Error::Stale);
        }
        reader.check(now_ms, false, false)?;
        Ok(PlanQualitySamplesPage {
            routing_observations,
            samples: rows,
            summary,
            next_cursor,
        })
    }
}

fn generations(connection: &Connection) -> rusqlite::Result<(u64, u64)> {
    connection.query_row(
        "SELECT (SELECT CAST(value AS INTEGER) FROM observation_meta WHERE key='plan_quality_generation'),
                (SELECT CAST(value AS INTEGER) FROM observation_meta WHERE key='query_visibility_generation')",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
}

pub(super) fn summary_sql() -> String {
    // Aggregate projected names once per stage; flattening repeats the correlated lookups.
    format!(
        "WITH quality AS MATERIALIZED ({SCOPE})
         SELECT plan_revision,
           CASE WHEN executed_branch_id IS NULL THEN selected_branch_id END,
           executed_branch_id,model_configuration_id,profile_digest,attribution,
           CASE WHEN COUNT(DISTINCT native_model)=1 THEN MAX(native_model) END,
           CASE WHEN COUNT(DISTINCT reasoning_profile_id)=1 THEN MAX(reasoning_profile_id) END,
           COUNT(reliable_score),COUNT(*)-COUNT(reliable_score),AVG(CASE WHEN attribution='single' THEN reliable_score END), execution_group, candidate_index, MAX(branch_policy)
         FROM quality GROUP BY plan_revision,
           CASE WHEN executed_branch_id IS NULL THEN selected_branch_id END,
           executed_branch_id,model_configuration_id,profile_digest,attribution,execution_group,candidate_index
         ORDER BY plan_revision DESC,executed_branch_id,model_configuration_id,profile_digest,attribution
         LIMIT 1001"
    )
}

fn summarize(
    transaction: &Transaction<'_>,
    reader: &ObservationReaderContext,
    query: &PlanQualitySamplesQuery,
    from_ms: i64,
    to_ms: i64,
    runs: &Option<String>,
) -> Result<PlanQualitySummary, ObservationV2Error> {
    let sql = summary_sql();
    let scope_params = named_params! {
        ":workspace": reader.workspace().as_str(), ":from": from_ms, ":to": to_ms,
        ":plan": query.plan_id, ":session": query.session_id,
        ":revision": query.plan_revision.and_then(|v| i64::try_from(v).ok()), ":runs": runs,
    };
    let models = transaction
        .prepare(&sql)?
        .query_map(scope_params, |row| {
            Ok(PlanQualityModelSummary {
                branch_policy: read_json(row.get(13)?)?,
                execution: PlanQualityExecutionIdentity {
                    group: row
                        .get::<_, Option<String>>(11)?
                        .map(|g| match g.as_str() {
                            "regular" => Ok(hiroute_domain::ExecutionGroupV1::Regular),
                            "primary" => Ok(hiroute_domain::ExecutionGroupV1::Primary),
                            _ => Err(rusqlite::Error::InvalidQuery),
                        })
                        .transpose()?,
                    candidate_index: row.get(12)?,
                    plan_revision: sql_u64(row.get(0)?)?,
                    selected_branch_id: row.get(1)?,
                    executed_branch_id: row.get(2)?,
                    model_configuration_id: row.get(3)?,
                    profile_digest: row.get(4)?,
                    attribution: attribution(row.get::<_, String>(5)?.as_str())?,
                },
                native_model: row.get(6)?,
                reasoning_profile_id: row.get(7)?,
                scored_stage_count: sql_u64(row.get(8)?)?,
                unrated_stage_count: sql_u64(row.get(9)?)?,
                average_score: row.get(10)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    if models.len() > 1000 {
        return Err(ObservationV2Error::Unavailable);
    }
    let session_count: i64 = transaction.query_row(
        &format!("WITH quality AS ({SCOPE}) SELECT COUNT(DISTINCT session_id) FROM quality"),
        scope_params,
        |row| row.get(0),
    )?;
    let revisions = transaction.prepare(
        &format!("WITH quality AS ({SCOPE}) SELECT DISTINCT plan_revision FROM quality ORDER BY plan_revision DESC"),
    )?.query_map(named_params! {
        ":workspace": reader.workspace().as_str(), ":from": from_ms, ":to": to_ms,
        ":plan": query.plan_id, ":session": query.session_id,
        ":revision": Option::<i64>::None, ":runs": runs,
    }, |row| sql_u64(row.get(0)?))?.collect::<Result<Vec<_>, _>>()?;
    Ok(PlanQualitySummary {
        scored_stage_count: models.iter().map(|m| m.scored_stage_count).sum(),
        unrated_stage_count: models.iter().map(|m| m.unrated_stage_count).sum(),
        session_count: sql_u64(session_count)?,
        models,
        available_revisions: revisions,
    })
}

/// Resolve a recorded stage for one selected request. This is evidence lookup,
/// never a new classifier call, and shares the caller's transaction/deadline.
pub(crate) fn routing_reason(
    db: &Connection,
    workspace: &str,
    session: &str,
    request: &str,
) -> Result<Option<PlanQualitySample>, ObservationV2Error> {
    use rusqlite::OptionalExtension;
    let mut result = db
        .query_row(
            &format!(
                "WITH quality AS ({SCOPE}) {SAMPLE_SELECT}
      WHERE q.session_id=:session
        AND ((q.history_membership_json IS NOT NULL AND EXISTS(SELECT 1 FROM json_each(q.history_membership_json,'$.request_ids') member WHERE member.value=:request))
          OR (q.history_membership_json IS NULL AND (q.first_request_id=:request OR q.last_request_id=:request OR q.segment_id=(
            SELECT t.segment_id FROM observation_analysis_turns_v1 t WHERE t.workspace_id=q.workspace_id
              AND t.request_id=:request AND t.partial=0))))
        AND EXISTS(SELECT 1 FROM logical_requests request WHERE request.workspace_id=q.workspace_id
          AND request.session_id=q.session_id AND request.request_id=:request)
      ORDER BY q.first_at_ms DESC,q.segment_id LIMIT 1"
            ),
            named_params! {":workspace":workspace,":from":0i64,":to":i64::MAX,
            ":plan":Option::<String>::None,":session":session,":revision":Option::<u64>::None,
            ":runs":Option::<String>::None,":request":request},
            sample,
        )
        .optional()?;
    if let Some(value) = result.as_mut() {
        let covered: bool = db.query_row(
            "SELECT assessment_membership_json IS NULL OR EXISTS(SELECT 1 FROM json_each(assessment_membership_json,'$.request_ids') member WHERE member.value=?3) FROM plan_quality_segments WHERE workspace_id=?1 AND segment_id=?2",
            rusqlite::params![workspace, value.segment_id, request], |row| row.get(0))?;
        if !covered {
            value.assessment = None;
        }
    }
    Ok(result)
}

pub(super) fn sample(row: &rusqlite::Row<'_>) -> rusqlite::Result<PlanQualitySample> {
    let assessment_event_id: Option<String> = row.get(21)?;
    let assessment = assessment_event_id
        .map(|event_id| {
            Ok::<_, rusqlite::Error>(PlanQualityAssessment {
                event_id,
                trigger_request_id: row.get(22)?,
                assessed_at_ms: row.get(23)?,
                target_from_turn_id: row.get(24)?,
                target_through_turn_id: row.get(25)?,
                target_from_ordinal: sql_u64(row.get(26)?)?,
                target_through_ordinal: sql_u64(row.get(27)?)?,
                score: row.get(28)?,
                partial: row.get(29)?,
                reason: row.get(30)?,
                evidence_available: row.get(31)?,
            })
        })
        .transpose()?;
    Ok(PlanQualitySample {
        selection: read_json(row.get(36)?)?,
        branch_execution: read_json(row.get(34)?)?,
        upgrade: read_json(row.get(35)?)?,
        segment_id: row.get(0)?,
        session_id: row.get(1)?,
        plan_id: row.get(2)?,
        plan_revision: sql_u64(row.get(3)?)?,
        selected_branch_id: row.get(4)?,
        executed_branch_id: row.get(5)?,
        model_configuration_id: row.get(6)?,
        profile_digest: row.get(7)?,
        attribution: attribution(row.get::<_, String>(8)?.as_str())?,
        first_turn_id: row.get(9)?,
        first_turn_ordinal: sql_u64(row.get(10)?)?,
        last_observed_turn_id: row.get(11)?,
        last_observed_turn_ordinal: sql_u64(row.get(12)?)?,
        first_at_ms: row.get(13)?,
        last_at_ms: row.get(14)?,
        history_partial: row.get(15)?,
        first_request_id: row.get(16)?,
        last_request_id: row.get(17)?,
        execution_evidence_available: row.get(18)?,
        task_id: row.get(19)?,
        run_id: row.get(20)?,
        assessment,
        native_model: row.get(32)?,
        reasoning_profile_id: row.get(33)?,
    })
}

fn validate(query: &PlanQualitySamplesQuery) -> Result<(), ObservationV2Error> {
    let execution_invalid = query.execution.as_ref().is_some_and(|e| {
        e.plan_revision == 0
            || i64::try_from(e.plan_revision).is_err()
            || e.executed_branch_id.is_some() == e.selected_branch_id.is_some()
            || [
                &e.selected_branch_id,
                &e.executed_branch_id,
                &e.model_configuration_id,
                &e.profile_digest,
            ]
            .into_iter()
            .flatten()
            .any(|value| !identifier(value))
            || query
                .plan_revision
                .is_some_and(|revision| revision != e.plan_revision)
    });
    let trust_invalid = query.trust_row.as_ref().is_some_and(|row| {
        !identifier(&row.branch_id)
            || [
                &row.model_configuration_id,
                &row.reasoning_profile_id,
                &row.executor,
            ]
            .into_iter()
            .flatten()
            .any(|v| !identifier(v))
    });
    if trust_invalid
        || query.segment_ids.len() > 200
        || query.segment_ids.iter().any(|id| !identifier(id))
        || query.executor.as_ref().is_some_and(|id| !identifier(id))
        || query.plan_id.is_none() && query.session_id.is_none()
        || query.limit == 0
        || query.limit > 200
        || query.plan_revision == Some(0)
        || execution_invalid
        || query.unrated_only
            && (query.score_gt.is_some() || query.score_lt.is_some() || query.competence.is_some())
        || query.from_ms.is_some_and(|value| value < 0)
        || query.to_ms.is_some_and(|value| value < 0)
        || query
            .from_ms
            .zip(query.to_ms)
            .is_some_and(|(from, to)| from >= to)
        || [
            &query.plan_id,
            &query.session_id,
            &query.segment_id,
            &query.model_configuration_id,
        ]
        .into_iter()
        .flatten()
        .any(|value| !identifier(value))
        || [query.score_gt, query.score_lt]
            .into_iter()
            .flatten()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
        || query
            .score_gt
            .zip(query.score_lt)
            .is_some_and(|(gt, lt)| gt >= lt)
        || query
            .plan_revision
            .is_some_and(|value| i64::try_from(value).is_err())
    {
        Err(ObservationV2Error::Invalid)
    } else {
        Ok(())
    }
}

fn attribution(value: &str) -> rusqlite::Result<AgentTurnAttributionV1> {
    match value {
        "single" => Ok(AgentTurnAttributionV1::Single),
        "mixed" => Ok(AgentTurnAttributionV1::Mixed),
        "unknown" => Ok(AgentTurnAttributionV1::Unknown),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn sql_u64(value: i64) -> rusqlite::Result<u64> {
    value.try_into().map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })
}

fn read_json<T: serde::de::DeserializeOwned>(value: Option<String>) -> rusqlite::Result<Option<T>> {
    value
        .map(|value| serde_json::from_str(&value).map_err(|_| rusqlite::Error::InvalidQuery))
        .transpose()
}
