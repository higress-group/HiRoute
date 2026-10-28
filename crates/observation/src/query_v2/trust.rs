//! A complete bounded population in one read transaction. No paging or sampling
//! is allowed to silently bias replay or coverage.
use super::plan_quality::{SAMPLE_SELECT, SCOPE, sample};
use super::{ObservationReaderContext, ObservationV2Error, QueryDeadline, identifier};
use crate::LocalObservationStore;
use hiroute_domain::{TrustObservedStageV1, TrustPopulationQueryV1, TrustPopulationV1};
use rusqlite::{Connection, OpenFlags, named_params};

impl LocalObservationStore {
    pub fn observed_trust_generation(
        &self,
        reader: &ObservationReaderContext,
        now_ms: i64,
    ) -> Result<(u64, u64), ObservationV2Error> {
        reader.check(now_ms, false, false)?;
        let _permit = self.query_permit()?;
        let connection =
            Connection::open_with_flags(&self.activity_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let _deadline = QueryDeadline::start_for(&connection, reader)?;
        Ok(connection.query_row("SELECT (SELECT CAST(value AS INTEGER) FROM observation_meta WHERE key='plan_quality_generation'), (SELECT CAST(value AS INTEGER) FROM observation_meta WHERE key='query_visibility_generation')", [], |row| Ok((row.get(0)?,row.get(1)?)))?)
    }
    pub fn observed_trust_population(
        &self,
        reader: &ObservationReaderContext,
        query: &TrustPopulationQueryV1,
        now_ms: i64,
    ) -> Result<TrustPopulationV1, ObservationV2Error> {
        self.trust_population(reader, query, now_ms, false)
    }
    pub fn observed_trust_suggestion_population(
        &self,
        reader: &ObservationReaderContext,
        query: &TrustPopulationQueryV1,
        now_ms: i64,
    ) -> Result<TrustPopulationV1, ObservationV2Error> {
        self.trust_population(reader, query, now_ms, true)
    }
    fn trust_population(
        &self,
        reader: &ObservationReaderContext,
        query: &TrustPopulationQueryV1,
        now_ms: i64,
        lean: bool,
    ) -> Result<TrustPopulationV1, ObservationV2Error> {
        reader.check(now_ms, false, false)?;
        if query.active_revision == 0
            || i64::try_from(query.active_revision).is_err()
            || !identifier(&query.plan_id)
            || query.branch_id.as_ref().is_some_and(|id| !identifier(id))
            || query
                .revision
                .is_some_and(|n| n == 0 || i64::try_from(n).is_err())
            || query.from_ms.is_some_and(|v| v < 0)
            || query.to_ms.is_some_and(|v| v < 0)
        {
            return Err(ObservationV2Error::Invalid);
        }
        let retained_from = now_ms
            .saturating_sub(crate::managed_text::RETENTION_MS)
            .saturating_add(1)
            .max(0);
        let from_ms = query.from_ms.unwrap_or(retained_from).max(retained_from);
        let to_ms = query.to_ms.unwrap_or_else(|| now_ms.saturating_add(1));
        if from_ms >= to_ms {
            return Err(ObservationV2Error::Invalid);
        }
        let _permit = self.query_permit()?;
        let mut connection =
            Connection::open_with_flags(&self.activity_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let _deadline = QueryDeadline::start_for(&connection, reader)?;
        let transaction = connection.transaction()?;
        let generation: u64 = transaction.query_row("SELECT CAST(value AS INTEGER) FROM observation_meta WHERE key='plan_quality_generation'", [], |row| row.get(0))?;
        let runs = reader
            .allowed_runs()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|_| ObservationV2Error::Invalid)?;
        let projection = SAMPLE_SELECT.replace("FROM quality q", ", CASE WHEN l.conflicted=0 THEN json_extract(l.body_json,'$.harness_id') WHEN l.request_id IS NULL THEN (SELECT s.agent_id FROM sessions s WHERE s.workspace_id=q.workspace_id AND s.session_id=q.session_id) END AS executor, EXISTS(SELECT 1 FROM plan_quality_segments later WHERE later.workspace_id=q.workspace_id AND later.session_id=q.session_id AND later.last_observed_turn_ordinal>q.last_observed_turn_ordinal) AS has_later_turn,
            (SELECT json_group_array(json_object('duration_ms',r.finished_at_ms-r.started_at_ms,'output',json_extract(u.usage_json,'$.output')))
             FROM logical_requests r
             JOIN valuation_requests_v2 v ON v.workspace_id=r.workspace_id AND v.request_id=r.request_id
             LEFT JOIN valuation_attempt_inputs_v2 u ON u.workspace_id=v.workspace_id AND u.request_id=v.request_id AND u.ordinal=v.accepted_ordinal
             WHERE r.workspace_id=q.workspace_id AND r.session_id=q.session_id AND v.plan_id=q.plan_id
               AND json_extract(r.route_json,'$.revision')=q.plan_revision
               AND ((q.history_membership_json IS NOT NULL AND EXISTS(SELECT 1 FROM json_each(q.history_membership_json,'$.request_ids') member WHERE member.value=r.request_id))
                 OR (q.history_membership_json IS NULL
                   AND r.started_at_ms >= (SELECT first.started_at_ms FROM logical_requests first WHERE first.workspace_id=q.workspace_id AND first.request_id=q.first_request_id)
                   AND r.started_at_ms <= (SELECT last.started_at_ms FROM logical_requests last WHERE last.workspace_id=q.workspace_id AND last.request_id=q.last_request_id)))
               AND r.finished_at_ms>=r.started_at_ms) AS performance
            FROM quality q");
        let projection = if lean {
            SAMPLE_SELECT.replace("FROM quality q", ", NULL AS executor, EXISTS(SELECT 1 FROM plan_quality_segments later WHERE later.workspace_id=q.workspace_id AND later.session_id=q.session_id AND later.last_observed_turn_ordinal>q.last_observed_turn_ordinal) AS has_later_turn, '[]' AS performance FROM quality q")
        } else {
            projection
        };
        let scope = if lean {
            format!(
                "SELECT q.*, retired_assessment.request_id IS NOT NULL AS assessment_retired, (q.assessment_membership_json IS NULL OR NOT EXISTS(SELECT 1 FROM json_each(q.history_membership_json,'$.request_ids') actual WHERE NOT EXISTS(SELECT 1 FROM json_each(q.assessment_membership_json,'$.request_ids') assessed WHERE assessed.value=actual.value))) AS assessment_covers_stage, NULL AS native_model, NULL AS reasoning_profile_id {}",
                &SCOPE[SCOPE
                    .find("FROM plan_quality_segments q")
                    .expect("scope owner")..]
            )
        } else {
            SCOPE.to_owned()
        };
        let population_scope = scope.replace("q.last_at_ms>=:from AND q.last_at_ms<:to", "q.last_at_ms>=:retained AND q.last_at_ms<:now AND (q.plan_revision=:active OR (q.last_at_ms>=:from AND q.last_at_ms<:to AND (:revision IS NULL OR q.plan_revision=:revision)))").replace("AND (:revision IS NULL OR q.plan_revision=:revision)\n", "\n");
        let sql = format!(
            "WITH quality AS ({population_scope}) {projection} WHERE (:branch IS NULL OR COALESCE(q.executed_branch_id,q.selected_branch_id)=:branch) ORDER BY q.last_at_ms,q.segment_id LIMIT 50001"
        );
        let stages = transaction
            .prepare(&sql).map_err(scope_error)?
            .query_map(
                named_params! {
                    ":workspace": reader.workspace().as_str(), ":from": from_ms, ":to": to_ms,
                    ":active": query.active_revision, ":retained": retained_from, ":now": now_ms.saturating_add(1),
                    ":plan": query.plan_id, ":session": Option::<String>::None,
                    ":revision": query.revision, ":runs": runs, ":branch": query.branch_id,
                },
                |row| {
                    let performance: Vec<Performance> = serde_json::from_str(&row.get::<_, String>(39)?).map_err(|_| rusqlite::Error::InvalidQuery)?;
                    Ok(TrustObservedStageV1 {
                        sample: sample(row)?,
                        executor: row.get::<_, Option<String>>(37)?.filter(|s| !s.is_empty()),
                        has_later_turn: row.get(38)?,
                        request_durations_ms: performance.iter().map(|p| p.duration_ms).collect(),
                        output_tokens_per_second: performance.iter().filter(|p| p.duration_ms > 0).filter_map(|p| p.output.map(|tokens| tokens as f64 * 1000.0 / p.duration_ms as f64)).collect(),
                    })
                },
            ).map_err(scope_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(scope_error)?;
        if stages.len() > 50_000 {
            return Err(ObservationV2Error::ScopeTooLarge);
        }
        let active_stages = stages
            .iter()
            .filter(|s| s.sample.plan_revision == query.active_revision)
            .cloned()
            .collect();
        let stages = stages
            .into_iter()
            .filter(|s| {
                s.sample.last_at_ms >= from_ms
                    && s.sample.last_at_ms < to_ms
                    && query.revision.is_none_or(|r| s.sample.plan_revision == r)
            })
            .collect();
        transaction.commit()?;
        reader.check(now_ms, false, false)?;
        Ok(TrustPopulationV1 {
            active_stages,
            retained_from_ms: retained_from,
            stages,
            generation,
            from_ms,
            to_ms,
        })
    }
}
fn scope_error(error: rusqlite::Error) -> ObservationV2Error {
    if matches!(error, rusqlite::Error::SqliteFailure(ref code, _) if code.code == rusqlite::ErrorCode::OperationInterrupted)
    {
        ObservationV2Error::ScopeTooLarge
    } else {
        error.into()
    }
}

#[derive(serde::Deserialize)]
struct Performance {
    duration_ms: u64,
    output: Option<u64>,
}
