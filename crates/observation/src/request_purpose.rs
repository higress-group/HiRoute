//! Bounded affirmative request-purpose evidence shared by views and trial scope.
use hiroute_domain::{CanonicalDigest, ExecutionFactEnvelopeV1, ExecutionFactV1};
use rusqlite::{Connection, params};

pub(crate) const QUERY_CARRIER_BYTES: usize = 1024 * 1024;
const CARRIER_BYTES: usize = 64 * 1024;

pub(crate) fn native_auxiliary(
    transaction: &Connection,
    workspace: &str,
    session: &str,
    request: &str,
    remaining_bytes: &mut usize,
) -> Option<bool> {
    let limit = (*remaining_bytes).min(CARRIER_BYTES);
    if limit == 0 {
        return None;
    }
    // The existing safe-route index bounds this to two matching metadata rows.
    // Duplicate or missing witnesses remain unknown; read at most one bounded
    // carrier and never reconstruct a deleted purpose from a prompt or receipt.
    let (rows, body, digest): (i64, Option<String>, Option<String>) = transaction.query_row(
        "WITH routes AS MATERIALIZED (
            SELECT original_digest FROM observation_safe_facts_v2 INDEXED BY observation_safe_route_payloads
            WHERE workspace_id=?1 AND request_id=?2
              AND CASE WHEN json_valid(body_json) THEN json_extract(body_json,'$.event_kind') ELSE 'route_decision' END='route_decision'
            LIMIT 2
         ) SELECT (SELECT COUNT(*) FROM routes),
           CASE WHEN (SELECT COUNT(*) FROM routes)=1 THEN (
             SELECT CASE WHEN p.workspace_id=?1 AND p.request_id=?2 AND p.session_id=?3
               AND length(CAST(p.body_json AS BLOB))<=?4
               AND EXISTS(SELECT 1 FROM execution_fact_events e INDEXED BY execution_facts_request_identity WHERE e.workspace_id=?1 AND e.request_id=?2 AND e.envelope_digest=r.original_digest)
               THEN p.body_json END
             FROM routes r LEFT JOIN observation_sensitive_payloads_v2 p ON p.id='fact:'||r.original_digest
           ) END,
           (SELECT original_digest FROM routes LIMIT 1)",
        params![workspace, request, session, limit as u64],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).ok()?;
    if rows != 1 {
        return None;
    }
    let body = body?;
    *remaining_bytes = remaining_bytes.saturating_sub(body.len());
    let value: serde_json::Value = serde_json::from_str(&body).ok()?;
    if CanonicalDigest::of(&value).ok()?.as_str() != digest.as_deref()? {
        return None;
    }
    let envelope: ExecutionFactEnvelopeV1 = serde_json::from_value(value).ok()?;
    if envelope.correlation.workspace_id.as_str() != workspace
        || envelope.correlation.conversation_id.as_str() != session
        || envelope.correlation.request_id.as_str() != request
        || envelope.validate_persisted_contract().is_err()
    {
        return None;
    }
    match envelope.fact {
        ExecutionFactV1::RouteDecision(route) if route.native_auxiliary => Some(true),
        _ => None,
    }
}
