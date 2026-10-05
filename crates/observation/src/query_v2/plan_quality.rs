use hiroute_domain::{
    AgentTurnAttributionV1, CanonicalDigest, PlanQualityAssessment, PlanQualityExecutionIdentity,
    PlanQualityModelSummary, PlanQualitySample, PlanQualitySamplesPage, PlanQualitySamplesQuery,
    PlanQualitySummary,
};
use rusqlite::{Connection, OpenFlags, Transaction, named_params};
use serde::{Deserialize, Serialize};

use super::{ObservationReaderContext, ObservationV2Error, QueryDeadline, identifier};
use crate::LocalObservationStore;

/// Both projections share the same scope and transaction. Detail filters never
/// enter this CTE, so pagination and drill-down cannot change the full-scope mean.
const SCOPE: &str = "
    SELECT q.*,
      COALESCE(
        (SELECT a.model_id FROM observation_attempt_models_v2 a
         JOIN valuation_requests_v2 v ON v.workspace_id=a.workspace_id
           AND v.request_id=a.request_id AND v.accepted_ordinal=a.ordinal
         JOIN logical_requests r ON r.workspace_id=a.workspace_id AND r.request_id=a.request_id
         WHERE a.workspace_id=q.workspace_id AND a.request_id=q.last_request_id
           AND r.session_id=q.session_id LIMIT 1),
        (SELECT a.model_id FROM observation_attempt_models_v2 a
         JOIN valuation_requests_v2 v ON v.workspace_id=a.workspace_id
           AND v.request_id=a.request_id AND v.accepted_ordinal=a.ordinal
         JOIN logical_requests r ON r.workspace_id=a.workspace_id AND r.request_id=a.request_id
         WHERE a.workspace_id=q.workspace_id AND a.request_id=q.first_request_id
           AND r.session_id=q.session_id LIMIT 1)
      ) AS native_model,
      (SELECT json_extract(p.body_json,'$.fact.reasoning_profile_id')
       FROM execution_fact_events e
       JOIN observation_sensitive_payloads_v2 p ON p.id='fact:'||e.envelope_digest
       JOIN logical_requests r ON r.workspace_id=e.workspace_id AND r.request_id=e.request_id
       WHERE e.workspace_id=q.workspace_id AND r.session_id=q.session_id
         AND e.request_id IN(q.last_request_id,q.first_request_id)
         AND json_extract(p.body_json,'$.fact.kind')='candidate_decision'
         AND json_extract(p.body_json,'$.fact.model_configuration_id')=q.model_configuration_id
         AND json_extract(p.body_json,'$.fact.profile_digest')=q.profile_digest
       ORDER BY e.sequence DESC LIMIT 1
      ) AS reasoning_profile_id
    FROM plan_quality_segments q
    JOIN sessions s ON s.workspace_id=q.workspace_id AND s.session_id=q.session_id
    LEFT JOIN observation_run_links l ON l.workspace_id=q.workspace_id
      AND l.request_id=q.first_request_id
    WHERE q.workspace_id=:workspace AND q.last_at_ms>=:from AND q.last_at_ms<:to
      AND q.last_at_ms IS NOT NULL AND q.selected_branch_id IS NOT NULL
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
        let generation: u64 = transaction.query_row(
            "SELECT CAST(value AS INTEGER) FROM observation_meta
             WHERE key='plan_quality_generation'",
            [],
            |row| row.get(0),
        )?;
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
        let summary = summarize(&transaction, reader, query, from_ms, to_ms, &runs)?;
        let execution = query
            .execution
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|_| ObservationV2Error::Invalid)?;
        let mut rows = {
            let sql = format!(
                "WITH quality AS ({SCOPE})
                 SELECT q.segment_id,q.session_id,q.plan_id,q.plan_revision,
                        q.selected_branch_id,q.executed_branch_id,q.model_configuration_id,
                        q.profile_digest,q.attribution,q.first_turn_id,q.first_turn_ordinal,
                        q.last_observed_turn_id,q.last_observed_turn_ordinal,q.first_at_ms,
                        q.last_at_ms,q.history_partial,q.first_request_id,q.last_request_id,
                        EXISTS(SELECT 1 FROM logical_requests first_req
                          WHERE first_req.workspace_id=q.workspace_id
                            AND first_req.session_id=q.session_id
                            AND first_req.request_id=q.first_request_id)
                          AND EXISTS(SELECT 1 FROM logical_requests last_req
                          WHERE last_req.workspace_id=q.workspace_id
                            AND last_req.session_id=q.session_id
                            AND last_req.request_id=q.last_request_id),
                        CASE WHEN COALESCE(l.conflicted,0)=0 THEN json_extract(l.body_json,'$.task_id') END,
                        CASE WHEN COALESCE(l.conflicted,0)=0 THEN l.run_id END,
                        q.assessment_event_id,q.assessment_trigger_request_id,q.assessed_at_ms,
                        q.target_from_turn_id,q.target_through_turn_id,q.target_from_ordinal,
                        q.target_through_ordinal,q.score,q.assessment_partial,
                        CASE WHEN q.reason_present=1
                          THEN json_extract(p.body_json,'$.fact.reason') END,
                        q.assessment_event_id IS NOT NULL AND EXISTS(
                          SELECT 1 FROM logical_requests trigger_req
                          WHERE trigger_req.workspace_id=q.workspace_id
                            AND trigger_req.session_id=q.session_id
                            AND trigger_req.request_id=q.assessment_trigger_request_id),
                        q.native_model,q.reasoning_profile_id
                 FROM quality q
                 LEFT JOIN observation_run_links l ON l.workspace_id=q.workspace_id
                    AND l.request_id=q.first_request_id
                 LEFT JOIN execution_fact_events e ON e.workspace_id=q.workspace_id
                    AND e.event_id=q.assessment_event_id
                 LEFT JOIN observation_sensitive_payloads_v2 p
                    ON p.id='fact:'||e.envelope_digest
                 WHERE (:segment IS NULL OR q.segment_id=:segment)
                   AND (:model IS NULL OR q.model_configuration_id=:model)
                   AND (:unrated=0 OR q.assessment_event_id IS NULL)
                   AND (:gt IS NULL OR q.score>:gt)
                   AND (:lt IS NULL OR q.score<:lt)
                   AND (:execution IS NULL OR (
                     q.plan_revision=json_extract(:execution,'$.plan_revision')
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
                        ":model": query.model_configuration_id, ":unrated": query.unrated_only,
                        ":gt": query.score_gt, ":lt": query.score_lt,
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
        let current: u64 = connection.query_row(
            "SELECT CAST(value AS INTEGER) FROM observation_meta
             WHERE key='plan_quality_generation'",
            [],
            |row| row.get(0),
        )?;
        if current != generation {
            return Err(ObservationV2Error::Stale);
        }
        reader.check(now_ms, false, false)?;
        Ok(PlanQualitySamplesPage {
            samples: rows,
            summary,
            next_cursor,
        })
    }
}

fn summarize(
    transaction: &Transaction<'_>,
    reader: &ObservationReaderContext,
    query: &PlanQualitySamplesQuery,
    from_ms: i64,
    to_ms: i64,
    runs: &Option<String>,
) -> Result<PlanQualitySummary, ObservationV2Error> {
    let sql = format!(
        "WITH quality AS ({SCOPE})
         SELECT plan_revision,
           CASE WHEN executed_branch_id IS NULL THEN selected_branch_id END,
           executed_branch_id,model_configuration_id,profile_digest,attribution,
           CASE WHEN COUNT(DISTINCT native_model)=1 THEN MAX(native_model) END,
           CASE WHEN COUNT(DISTINCT reasoning_profile_id)=1 THEN MAX(reasoning_profile_id) END,
           COUNT(score),COUNT(*)-COUNT(score),AVG(score)
         FROM quality GROUP BY plan_revision,
           CASE WHEN executed_branch_id IS NULL THEN selected_branch_id END,
           executed_branch_id,model_configuration_id,profile_digest,attribution
         ORDER BY plan_revision DESC,executed_branch_id,model_configuration_id,profile_digest,attribution
         LIMIT 1001"
    );
    let scope_params = named_params! {
        ":workspace": reader.workspace().as_str(), ":from": from_ms, ":to": to_ms,
        ":plan": query.plan_id, ":session": query.session_id,
        ":revision": query.plan_revision.and_then(|v| i64::try_from(v).ok()), ":runs": runs,
    };
    let models = transaction
        .prepare(&sql)?
        .query_map(scope_params, |row| {
            Ok(PlanQualityModelSummary {
                execution: PlanQualityExecutionIdentity {
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

fn sample(row: &rusqlite::Row<'_>) -> rusqlite::Result<PlanQualitySample> {
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
    if query.plan_id.is_none() && query.session_id.is_none()
        || query.limit == 0
        || query.limit > 200
        || query.plan_revision == Some(0)
        || execution_invalid
        || query.unrated_only && (query.score_gt.is_some() || query.score_lt.is_some())
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
