//! Bounded request-local metadata. Optional evidence never scans session history.
use hiroute_domain::{
    CanonicalDigest, ExecutionFactEnvelopeV1, ExecutionFactV1, RequestInputSelectionV1,
};
use rusqlite::{Transaction, params};

const METADATA_ROWS: usize = 128;
const CARRIER_BYTES: usize = 64 * 1024;

#[cfg(test)]
#[path = "input_selection_tests.rs"]
mod tests;

pub(super) fn read(
    tx: &Transaction<'_>,
    workspace: &str,
    request: &str,
    now_ms: i64,
) -> Option<RequestInputSelectionV1> {
    let (session, count, body, digest): (String, i64, Option<String>, Option<String>) = tx.query_row(
        "WITH local AS MATERIALIZED (
           SELECT s.original_digest,s.body_json FROM observation_safe_facts_v2 s
           WHERE s.workspace_id=?1 AND s.request_id=?2 LIMIT ?3
         ), selected AS MATERIALIZED (
           SELECT original_digest FROM local
           WHERE json_valid(body_json)
             AND json_extract(body_json,'$.event_kind')='request_input_selection' LIMIT 2
         )
         SELECT r.session_id,(SELECT COUNT(*) FROM local),
           CASE WHEN (SELECT COUNT(*) FROM selected)=1 THEN (
             SELECT p.body_json FROM selected s JOIN observation_sensitive_payloads_v2 p
               ON p.id='fact:'||s.original_digest
             WHERE p.workspace_id=?1 AND p.request_id=?2 AND p.session_id=r.session_id
               AND length(CAST(p.body_json AS BLOB))<=?5
               AND EXISTS(SELECT 1 FROM execution_fact_events e
                 WHERE e.workspace_id=?1 AND e.request_id=?2 AND e.envelope_digest=s.original_digest)
           ) END,
           CASE WHEN (SELECT COUNT(*) FROM selected)=1
             THEN (SELECT original_digest FROM selected LIMIT 1) END
         FROM logical_requests r WHERE r.workspace_id=?1 AND r.request_id=?2
           AND r.started_at_ms>?4",
        params![workspace, request, METADATA_ROWS + 1,
            now_ms.saturating_sub(crate::managed_text::RETENTION_MS), CARRIER_BYTES],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
    ).ok()?;
    if count > METADATA_ROWS as i64 {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(&body?).ok()?;
    if CanonicalDigest::of(&value).ok()?.as_str() != digest.as_deref()? {
        return None;
    }
    let envelope: ExecutionFactEnvelopeV1 = serde_json::from_value(value).ok()?;
    if envelope.validate_persisted_contract().is_err()
        || envelope.correlation.workspace_id.as_str() != workspace
        || envelope.correlation.request_id.as_str() != request
        || envelope.correlation.conversation_id.as_str() != session
    {
        return None;
    }
    match envelope.fact {
        ExecutionFactV1::RequestInputSelection(selection) => Some(selection),
        _ => None,
    }
}
