use super::*;
use hiroute_domain::{
    CanonicalDigest, ContentCompleteness, ObservationSessionCorrelationKindV1,
    ObservationSessionPageV2, ObservationSessionSummaryV2,
};
use rusqlite::{Connection, OpenFlags, params};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionCursor {
    schema: String,
    binding: String,
    visibility: u64,
    watermark: i64,
    last_at: i64,
    last_id: String,
}

impl crate::LocalObservationStore {
    /// Group only matching authorized requests, never an unscoped session's
    /// counts or most recent model. Aggregation and keyset paging share one
    /// read snapshot and deadline; no request-page loop builds the list.
    pub fn observed_sessions(
        &self,
        reader: &ObservationReaderContext,
        query: &ObservationRequestQuery,
        now_ms: i64,
    ) -> Result<ObservationSessionPageV2, ObservationV2Error> {
        reader.check(now_ms, false, false)?;
        if query.from_ms < 0
            || query.from_ms >= query.to_ms
            || query.limit == 0
            || query.limit > 200
            || [
                &query.session_id,
                &query.request_id,
                &query.agent_id,
                &query.plan_id,
                &query.native_model,
                &query.outcome,
            ]
            .into_iter()
            .flatten()
            .any(|id| !identifier(id))
        {
            return Err(ObservationV2Error::Invalid);
        }
        let _permit = self.query_permit()?;
        let mut normalized = query.clone();
        normalized.cursor = None;
        let binding = CanonicalDigest::of(&(reader.binding()?, normalized))
            .map_err(|_| ObservationV2Error::Invalid)?
            .to_string();
        let mut connection =
            Connection::open_with_flags(&self.activity_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let _deadline = QueryDeadline::start_for(&connection, reader)?;
        let tx = connection.transaction()?;
        let visibility = tx.query_row("SELECT CAST(value AS INTEGER) FROM observation_meta WHERE key='query_visibility_generation'", [], |row| row.get(0))?;
        let cursor = match &query.cursor {
            Some(encoded) => {
                let cursor: SessionCursor = self.decode_observation_cursor(encoded)?;
                if cursor.schema != "sessions/v2"
                    || cursor.binding != binding
                    || cursor.visibility != visibility
                {
                    return Err(ObservationV2Error::Stale);
                }
                cursor
            }
            None => SessionCursor {
                schema: "sessions/v2".into(),
                binding,
                visibility,
                watermark: tx.query_row(
                    "SELECT COALESCE(MAX(rowid),0) FROM logical_requests",
                    [],
                    |r| r.get(0),
                )?,
                last_at: i64::MAX,
                last_id: String::new(),
            },
        };
        let runs = reader
            .allowed_runs()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|_| ObservationV2Error::Invalid)?;
        let mut sessions = {
            let request_state = crate::content::completeness::REQUEST_STATE;
            let sql = format!(
                "{}, visible AS (
                 SELECT r.session_id,s.agent_id,r.started_at_ms,r.session_scope,
                   r.correlation_provenance,l.run_id,l.conflicted,
                   CASE WHEN s.content_completeness IN ('deleted','expired') THEN s.content_completeness
                     ELSE {request_state} END AS content_state,
                   (SELECT COUNT(DISTINCT a.model_id) FROM observation_attempt_models_v2 a WHERE a.workspace_id=r.workspace_id AND a.request_id=r.request_id) AS models,
                   EXISTS(SELECT 1 FROM turn_changes t WHERE t.request_id=r.request_id AND t.changed=1) AS turn_change
                 FROM logical_requests r JOIN sessions s ON s.workspace_id=r.workspace_id AND s.session_id=r.session_id
                 LEFT JOIN observation_run_links l ON l.workspace_id=r.workspace_id AND l.request_id=r.request_id
                 WHERE r.workspace_id=?1 AND r.started_at_ms>=?2 AND r.started_at_ms<?3 AND r.rowid<=?4
                   AND (?5 IS NULL OR r.session_id=?5) AND (?6 IS NULL OR s.agent_id=?6)
                   AND (?7 IS NULL OR EXISTS(SELECT 1 FROM valuation_requests_v2 v WHERE v.workspace_id=r.workspace_id AND v.request_id=r.request_id AND v.plan_id=?7))
                   AND (?8 IS NULL OR EXISTS(SELECT 1 FROM observation_attempt_models_v2 a WHERE a.workspace_id=r.workspace_id AND a.request_id=r.request_id AND a.model_id=?8))
                   AND (?9 IS NULL OR r.outcome=?9)
                   AND (?10 IS NULL OR EXISTS(SELECT 1 FROM observation_run_links l WHERE l.workspace_id=r.workspace_id AND l.request_id=r.request_id AND l.conflicted=0 AND l.run_id IN(SELECT value FROM json_each(?10))))
                   AND (?15 IS NULL OR r.request_id=?15)
                   AND (s.tombstone_reason IS NULL OR EXISTS(SELECT 1 FROM observation_tombstones t WHERE t.workspace_id=r.workspace_id AND t.session_id=r.session_id AND t.delete_scope='content_only'))
                 ), grouped AS (
                   SELECT session_id,agent_id,MIN(started_at_ms) AS first_at,MAX(started_at_ms) AS last_at,COUNT(*) AS requests,SUM(models>1) AS fallbacks,SUM(models=0) AS unknown_models,
                     CASE
                       WHEN SUM(CASE WHEN conflicted=1 THEN 1 ELSE 0 END)>0 THEN 0
                       WHEN SUM(CASE WHEN session_scope='conversation' AND correlation_provenance='protocol_state' AND run_id IS NOT NULL AND conflicted=0 THEN 0 ELSE 1 END)=0
                            AND COUNT(DISTINCT run_id)=1 THEN 2
                       WHEN SUM(CASE WHEN session_scope='conversation' AND correlation_provenance='agent_supplied' THEN 0 ELSE 1 END)=0 THEN 1
                       WHEN SUM(CASE WHEN session_scope='conversation' AND correlation_provenance='gateway_generated' THEN 0 ELSE 1 END)=0 THEN 3
                       WHEN SUM(CASE WHEN session_scope='request_scoped' AND correlation_provenance='unproven' THEN 0 ELSE 1 END)=0 THEN 4
                       ELSE 0
                     END AS correlation_kind,
                     CASE WHEN MAX(content_state='deleted')=1 THEN 'deleted'
                       WHEN MAX(content_state='expired')=1 THEN 'expired'
                       WHEN MAX(content_state='partial')=1 THEN 'partial'
                       WHEN MAX(content_state='unknown')=1 THEN 'unknown'
                       ELSE 'complete' END AS content_state
                   FROM visible WHERE (?11=0 OR models>1 OR turn_change) GROUP BY session_id,agent_id
                 ) SELECT session_id,agent_id,first_at,last_at,requests,fallbacks,unknown_models,correlation_kind,content_state FROM grouped
                 WHERE last_at<?12 OR (last_at=?12 AND session_id>?13)
                 ORDER BY last_at DESC,session_id LIMIT ?14", super::turns::cte(10, "?4"));
            let mut stmt = tx.prepare(&sql)?;
            stmt.query_map(
                params![
                    reader.workspace().as_str(),
                    query.from_ms.max(
                        now_ms
                            .saturating_sub(crate::managed_text::RETENTION_MS)
                            .saturating_add(1)
                    ),
                    query.to_ms,
                    cursor.watermark,
                    query.session_id,
                    query.agent_id,
                    query.plan_id,
                    query.native_model,
                    query.outcome,
                    runs,
                    query.only_model_switch,
                    cursor.last_at,
                    cursor.last_id,
                    u64::from(query.limit) + 1,
                    query.request_id
                ],
                |r| {
                    Ok(ObservationSessionSummaryV2 {
                        session_id: r.get(0)?,
                        agent_id: r.get(1)?,
                        first_request_at_ms: r.get(2)?,
                        last_request_at_ms: r.get(3)?,
                        request_count: r.get(4)?,
                        fallback_request_count: r.get(5)?,
                        unknown_model_request_count: r.get(6)?,
                        content_completeness: match r.get::<_, String>(8)?.as_str() {
                            "complete" => ContentCompleteness::Complete,
                            "partial" => ContentCompleteness::Partial,
                            "deleted" => ContentCompleteness::Deleted,
                            "expired" => ContentCompleteness::Expired,
                            _ => ContentCompleteness::Unknown,
                        },
                        correlation_kind: match r.get::<_, u8>(7)? {
                            1 => ObservationSessionCorrelationKindV1::AgentSupplied,
                            2 => ObservationSessionCorrelationKindV1::VerifiedWorker,
                            3 => ObservationSessionCorrelationKindV1::Inferred,
                            4 => ObservationSessionCorrelationKindV1::RequestScoped,
                            _ => ObservationSessionCorrelationKindV1::Unknown,
                        },
                    })
                },
            )?
            .collect::<Result<Vec<_>, _>>()?
        };
        let more = sessions.len() > usize::from(query.limit);
        sessions.truncate(usize::from(query.limit));
        let next_cursor = if more {
            let last = sessions.last().ok_or(ObservationV2Error::Unavailable)?;
            Some(self.encode_observation_cursor(&SessionCursor {
                last_at: last.last_request_at_ms,
                last_id: last.session_id.clone(),
                ..cursor
            })?)
        } else {
            None
        };
        tx.commit()?;
        check_visibility(&connection, visibility)?;
        Ok(ObservationSessionPageV2 {
            sessions,
            next_cursor,
        })
    }
}
