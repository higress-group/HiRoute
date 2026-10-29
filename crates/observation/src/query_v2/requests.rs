use super::*;
use crate::LocalObservationStore;
use hiroute_domain::{
    CanonicalDigest, ObservationRoutingContextStateV1, ObservationRoutingContextV1,
    ObservationSafeFactV2, RoutingReceiptV1, WorkspaceId,
};
use rusqlite::{Connection, OpenFlags, params};
use serde::{Deserialize, Serialize};

use crate::request_purpose as purpose;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    schema: u8,
    binding: String,
    visibility: u64,
    watermark: i64,
    last_started: i64,
    last_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManagedSensitiveMarker {
    managed_sensitive_ref: String,
}

impl LocalObservationStore {
    pub fn observed_requests(
        &self,
        reader: &ObservationReaderContext,
        query: &ObservationRequestQuery,
        now_ms: i64,
    ) -> Result<ObservationRequestPage, ObservationV2Error> {
        self.observed_requests_ordered(reader, query, now_ms, false)
    }

    /// Chronological request timeline for a selected session. Cursor direction
    /// is part of its binding; descending list cursors cannot be reused here.
    pub fn observed_timeline(
        &self,
        reader: &ObservationReaderContext,
        query: &ObservationRequestQuery,
        now_ms: i64,
    ) -> Result<ObservationRequestPage, ObservationV2Error> {
        if query.session_id.is_none() {
            return Err(ObservationV2Error::Invalid);
        }
        self.observed_requests_ordered(reader, query, now_ms, true)
    }

    fn observed_requests_ordered(
        &self,
        reader: &ObservationReaderContext,
        query: &ObservationRequestQuery,
        now_ms: i64,
        ascending: bool,
    ) -> Result<ObservationRequestPage, ObservationV2Error> {
        reader.check(now_ms, false, false)?;
        let _permit = self.query_permit()?;
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
            .any(|id| id.as_ref().is_some_and(|id| !identifier(id)))
        {
            return Err(ObservationV2Error::Invalid);
        }
        let binding = hiroute_domain::CanonicalDigest::of(&(
            reader.binding()?,
            ascending,
            query.from_ms,
            query.to_ms,
            &query.session_id,
            &query.request_id,
            query.limit,
            &query.agent_id,
            &query.plan_id,
            &query.native_model,
            &query.outcome,
            query.only_model_switch,
        ))
        .map_err(|_| ObservationV2Error::Invalid)?
        .to_string();
        let mut connection =
            Connection::open_with_flags(&self.activity_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let _deadline = super::QueryDeadline::start_for(&connection, reader)?;
        let transaction = connection.transaction()?;
        let visibility:u64=transaction.query_row(
            "SELECT CAST(value AS INTEGER) FROM observation_meta WHERE key='query_visibility_generation'",[],|row|row.get(0),
        )?;
        let cursor = if let Some(encoded) = &query.cursor {
            let cursor: Cursor = self.decode_observation_cursor(encoded)?;
            if cursor.schema != 2 || cursor.binding != binding || cursor.visibility != visibility {
                return Err(ObservationV2Error::Stale);
            }
            cursor
        } else {
            Cursor {
                schema: 2,
                binding,
                visibility,
                watermark: transaction.query_row(
                    "SELECT COALESCE(MAX(rowid),0) FROM logical_requests",
                    [],
                    |row| row.get(0),
                )?,
                last_started: if ascending { i64::MIN } else { i64::MAX },
                last_id: String::new(),
            }
        };
        let runs = reader
            .allowed_runs()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|_| ObservationV2Error::Invalid)?;
        let rows = {
            let sql = format!("{}{}", super::turns::cte(8, "?4"),
                "SELECT r.session_id,r.request_id,r.started_at_ms,r.outcome,r.receipt_id,
                    l.run_id,l.body_json,COALESCE(l.conflicted,0),
                    (SELECT COUNT(DISTINCT model_id) FROM observation_attempt_models_v2 a WHERE a.workspace_id=r.workspace_id AND a.request_id=r.request_id),
                    (SELECT a.model_id FROM observation_attempt_models_v2 a JOIN valuation_requests_v2 v ON v.workspace_id=a.workspace_id AND v.request_id=a.request_id AND v.accepted_ordinal=a.ordinal WHERE a.workspace_id=r.workspace_id AND a.request_id=r.request_id LIMIT 1)
                    ,(SELECT changed FROM turn_changes t WHERE t.request_id=r.request_id),
                    (SELECT previous_turn FROM turn_changes t WHERE t.request_id=r.request_id),
                    (SELECT previous_model FROM turn_changes t WHERE t.request_id=r.request_id),
                    (SELECT final_model FROM turn_changes t WHERE t.request_id=r.request_id),
                    CASE WHEN COALESCE(rsp.body_json,rr.body_json) IS NULL THEN NULL
                         WHEN length(COALESCE(rsp.body_json,rr.body_json))<=1048576
                         THEN COALESCE(rsp.body_json,rr.body_json) ELSE '' END,
                    rr.body_digest,
                    (SELECT f.body_json FROM observation_safe_facts_v2 f
                     WHERE f.workspace_id=r.workspace_id AND f.request_id=r.request_id
                     ORDER BY f.rowid LIMIT 1)
                 FROM logical_requests r LEFT JOIN observation_run_links l
                    ON l.workspace_id=r.workspace_id AND l.request_id=r.request_id
                 LEFT JOIN routing_receipts rr
                    ON rr.workspace_id=r.workspace_id AND rr.request_id=r.request_id
                 LEFT JOIN observation_sensitive_payloads_v2 rsp
                    ON rsp.id='receipt:'||rr.body_digest
                 JOIN sessions s ON s.workspace_id=r.workspace_id AND s.session_id=r.session_id
                 WHERE r.workspace_id=?1 AND r.started_at_ms>=?2 AND r.started_at_ms<?3
                    AND r.rowid<=?4 AND (?5 IS NULL OR r.session_id=?5)
                    AND ((?15=0 AND r.started_at_ms<?6) OR (?15=1 AND r.started_at_ms>?6) OR (r.started_at_ms=?6 AND r.request_id>?7))
                    AND (?8 IS NULL OR (l.conflicted=0 AND l.run_id IN (SELECT value FROM json_each(?8))))
                    AND (s.tombstone_reason IS NULL OR EXISTS(
                        SELECT 1 FROM observation_tombstones t WHERE t.workspace_id=r.workspace_id
                            AND t.session_id=r.session_id AND t.delete_scope='content_only'))
                    AND (?10 IS NULL OR s.agent_id=?10)
                    AND (?11 IS NULL OR EXISTS(SELECT 1 FROM valuation_requests_v2 v WHERE v.workspace_id=r.workspace_id AND v.request_id=r.request_id AND v.plan_id=?11))
                    AND (?12 IS NULL OR EXISTS(SELECT 1 FROM observation_attempt_models_v2 a WHERE a.workspace_id=r.workspace_id AND a.request_id=r.request_id AND a.model_id=?12))
                    AND (?13 IS NULL OR r.outcome=?13)
                    AND (?14=0 OR (SELECT COUNT(DISTINCT model_id) FROM observation_attempt_models_v2 a WHERE a.workspace_id=r.workspace_id AND a.request_id=r.request_id)>1 OR EXISTS(SELECT 1 FROM turn_changes t WHERE t.request_id=r.request_id AND t.changed=1))
                    AND (?16 IS NULL OR r.request_id=?16)
                 ORDER BY CASE WHEN ?15=0 THEN r.started_at_ms END DESC, CASE WHEN ?15=1 THEN r.started_at_ms END ASC,r.request_id LIMIT ?9",
            );
            let mut statement = transaction.prepare(&sql)?;
            let retained_from = now_ms
                .saturating_sub(crate::managed_text::RETENTION_MS)
                .saturating_add(1)
                .max(0);
            statement
                .query_map(
                    params![
                        reader.workspace().as_str(),
                        query.from_ms.max(retained_from),
                        query.to_ms,
                        cursor.watermark,
                        query.session_id,
                        cursor.last_started,
                        cursor.last_id,
                        runs,
                        u64::from(query.limit) + 1,
                        query.agent_id,
                        query.plan_id,
                        query.native_model,
                        query.outcome,
                        query.only_model_switch,
                        ascending,
                        query.request_id,
                    ],
                    |row| {
                        Ok((
                            ObservedRequestSummary {
                                session_id: row.get(0)?,
                                request_id: row.get(1)?,
                                started_at_ms: row.get(2)?,
                                outcome: row.get(3)?,
                                receipt_id: row.get(4)?,
                                run_id: row.get(5)?,
                                parent_context_ref: None,
                                native_turn_id: None,
                                relation_conflicted: row.get(7)?,
                                routing_context: ObservationRoutingContextV1::unavailable(),
                                native_auxiliary: None,
                                attempted_model_count: row.get(8)?,
                                within_request_fallback: {
                                    let count: u32 = row.get(8)?;
                                    (count > 0).then_some(count > 1)
                                },
                                final_native_model: row.get(9)?,
                                between_turn_model_change: row.get(10)?,
                                previous_native_turn_id: row.get(11)?,
                                previous_turn_model: row.get(12)?,
                                turn_final_native_model: row.get(13)?,
                            },
                            row.get::<_, Option<String>>(6)?,
                            row.get::<_, Option<String>>(14)?,
                            row.get::<_, Option<String>>(15)?,
                            row.get::<_, Option<String>>(16)?,
                        ))
                    },
                )?
                .collect::<Result<Vec<_>, _>>()?
        };
        let more = rows.len() > usize::from(query.limit);
        let mut requests = Vec::with_capacity(usize::from(query.limit));
        let mut purpose_bytes = purpose::QUERY_CARRIER_BYTES;
        for (mut request, link_body, receipt_body, receipt_digest, safe_fact_body) in
            rows.into_iter().take(usize::from(query.limit))
        {
            let link = link_body
                .as_deref()
                .and_then(|body| serde_json::from_str::<RunObservationLink>(body).ok())
                .filter(|link| {
                    link.workspace_id == *reader.workspace()
                        && link.request_id.as_str() == request.request_id
                        && request.run_id.as_deref() == Some(link.run_id.as_str())
                        && ObservationRoutingContextV1::recorded(
                            link.plan_id.clone(),
                            link.plan_revision.clone(),
                            None,
                        )
                        .valid()
                });
            if !request.relation_conflicted {
                if let Some(link) = &link {
                    request.native_turn_id = link.native_turn_id.clone();
                    request.parent_context_ref = link.parent_context_ref.clone();
                } else if link_body.is_some() {
                    request.run_id = None;
                }
            } else {
                request.run_id = None;
                request.parent_context_ref = None;
            }
            request.routing_context = observation_routing_context(
                &request,
                reader.workspace(),
                link.as_ref(),
                link_body.is_some() && link.is_none(),
                receipt_body.as_deref(),
                receipt_digest.as_deref(),
                safe_fact_body.as_deref(),
            );
            request.native_auxiliary = purpose::native_auxiliary(
                &transaction,
                reader.workspace().as_str(),
                &request.session_id,
                &request.request_id,
                &mut purpose_bytes,
            );
            requests.push(request);
        }
        let next_cursor = if more {
            let last = requests.last().ok_or(ObservationV2Error::Unavailable)?;
            Some(self.encode_observation_cursor(&Cursor {
                last_started: last.started_at_ms,
                last_id: last.request_id.clone(),
                ..cursor
            })?)
        } else {
            None
        };
        transaction.commit()?;
        super::check_visibility(&connection, visibility)?;
        reader.check(now_ms, false, false)?;
        Ok(ObservationRequestPage {
            requests,
            next_cursor,
        })
    }

    pub(crate) fn encode_observation_cursor<T: Serialize>(
        &self,
        cursor: &T,
    ) -> Result<String, ObservationV2Error> {
        let json = serde_json::to_string(cursor).map_err(|_| ObservationV2Error::Invalid)?;
        Ok(format!(
            "{}:{json}",
            self.authority.query_cursor_signature(json.as_bytes())
        ))
    }

    pub(crate) fn decode_observation_cursor<T: serde::de::DeserializeOwned>(
        &self,
        encoded: &str,
    ) -> Result<T, ObservationV2Error> {
        if encoded.len() > 4096 {
            return Err(ObservationV2Error::Invalid);
        }
        let (signature, json) = encoded.split_once(':').ok_or(ObservationV2Error::Invalid)?;
        if !self
            .authority
            .verify_query_cursor_signature(json.as_bytes(), signature)
        {
            return Err(ObservationV2Error::Invalid);
        }
        serde_json::from_str(json).map_err(|_| ObservationV2Error::Invalid)
    }
}

fn observation_routing_context(
    request: &ObservedRequestSummary,
    workspace: &WorkspaceId,
    link: Option<&RunObservationLink>,
    link_corrupt: bool,
    receipt_body: Option<&str>,
    receipt_digest: Option<&str>,
    safe_fact_body: Option<&str>,
) -> ObservationRoutingContextV1 {
    if request.relation_conflicted {
        return ObservationRoutingContextV1::conflicted();
    }
    if link_corrupt {
        return ObservationRoutingContextV1::unavailable();
    }
    let safe_context = match safe_fact_body {
        Some(body) => {
            let Ok(fact) = serde_json::from_str::<ObservationSafeFactV2>(body) else {
                return ObservationRoutingContextV1::unavailable();
            };
            match fact.routing_context {
                Some(context)
                    if context.valid()
                        && matches!(
                            context.state,
                            ObservationRoutingContextStateV1::Recorded
                                | ObservationRoutingContextStateV1::RecordedFixed
                        ) =>
                {
                    Some(context)
                }
                Some(_) => return ObservationRoutingContextV1::unavailable(),
                None => None,
            }
        }
        None => None,
    };
    let receipt_context = match receipt_body {
        Some(body) => {
            let receipt = match serde_json::from_str::<RoutingReceiptV1>(body) {
                Ok(receipt) => receipt,
                // A managed marker means sensitive receipt content was intentionally removed.
                // The safe fact projection above remains authoritative after that lifecycle.
                Err(_) if deleted_receipt_marker(body, receipt_digest) => {
                    return safe_context.map_or_else(
                        || link_context(link, link_corrupt),
                        |context| reconcile_link(context, link),
                    );
                }
                Err(_) => return ObservationRoutingContextV1::unavailable(),
            };
            if receipt.validate().is_err()
                || receipt.workspace_id != *workspace
                || receipt.session_id.as_str() != request.session_id
                || receipt.request_id.as_str() != request.request_id
            {
                return ObservationRoutingContextV1::unavailable();
            }
            Some(ObservationRoutingContextV1::from_execution_trust(
                &receipt.trust,
            ))
        }
        None => None,
    };
    let context = match (safe_context, receipt_context) {
        (Some(safe), Some(receipt)) if safe != receipt => {
            return ObservationRoutingContextV1::conflicted();
        }
        (Some(safe), _) => Some(safe),
        (_, Some(receipt)) => Some(receipt),
        (None, None) => None,
    };
    context.map_or_else(
        || link_context(link, link_corrupt),
        |context| reconcile_link(context, link),
    )
}

fn deleted_receipt_marker(body: &str, digest: Option<&str>) -> bool {
    let Some(digest) = digest.filter(|value| CanonicalDigest::parse(*value).is_ok()) else {
        return false;
    };
    serde_json::from_str::<ManagedSensitiveMarker>(body)
        .is_ok_and(|marker| marker.managed_sensitive_ref == format!("receipt:{digest}"))
}

fn reconcile_link(
    context: ObservationRoutingContextV1,
    link: Option<&RunObservationLink>,
) -> ObservationRoutingContextV1 {
    if link.is_some_and(|link| {
        context.plan_id.as_deref() != Some(link.plan_id.as_str())
            || context.plan_revision.as_deref() != Some(link.plan_revision.as_str())
    }) {
        ObservationRoutingContextV1::conflicted()
    } else {
        context
    }
}

fn link_context(
    link: Option<&RunObservationLink>,
    link_corrupt: bool,
) -> ObservationRoutingContextV1 {
    if link_corrupt {
        return ObservationRoutingContextV1::unavailable();
    }
    link.map_or_else(ObservationRoutingContextV1::unavailable, |link| {
        let context = ObservationRoutingContextV1::recorded(
            link.plan_id.clone(),
            link.plan_revision.clone(),
            None,
        );
        if context.valid() {
            context
        } else {
            ObservationRoutingContextV1::unavailable()
        }
    })
}
