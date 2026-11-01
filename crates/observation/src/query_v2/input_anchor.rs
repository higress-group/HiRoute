//! A request may refer to its already captured input. This one-hop reference
//! conveys no authority and cannot extend the source's retention or visibility.
use super::*;
use hiroute_domain::{LogicalRequestId, ObservationInputAnchorV1};
use rusqlite::{OptionalExtension, Transaction, params};

pub(super) fn read(
    tx: &Transaction<'_>,
    reader: &ObservationReaderContext,
    request: &LogicalRequestId,
    now: i64,
) -> Result<Option<ObservationInputAnchorV1>, ObservationV2Error> {
    let source:Option<(String,String)>=tx.query_row(
        "SELECT m.request_id,m.message_instance_id FROM observation_input_anchors_v6 a
         JOIN logical_requests r ON r.workspace_id=a.workspace_id AND r.request_id=a.request_id
         JOIN content_message_instances_v2 m ON m.workspace_id=a.workspace_id AND m.message_instance_id=a.source_message_id
         JOIN logical_requests s ON s.workspace_id=m.workspace_id AND s.request_id=m.request_id
         WHERE a.workspace_id=?1 AND a.request_id=?2 AND m.message_role='user'
           AND m.direction='request_input' AND r.session_id=s.session_id
           AND r.started_at_ms>?3 AND s.started_at_ms>?3
           AND NOT EXISTS(SELECT 1 FROM observation_capacity_pending_v5 p WHERE p.workspace=r.workspace_id AND p.request=r.request_id)
           AND NOT EXISTS(SELECT 1 FROM observation_request_tombstones_v2 t WHERE t.workspace_id=r.workspace_id AND t.request_id=r.request_id)
           AND NOT EXISTS(SELECT 1 FROM observation_capacity_pending_v5 p WHERE p.workspace=s.workspace_id AND p.request=s.request_id)
           AND NOT EXISTS(SELECT 1 FROM observation_request_tombstones_v2 t WHERE t.workspace_id=s.workspace_id AND t.request_id=s.request_id)
           AND EXISTS(SELECT 1 FROM content_instances_v2 c WHERE c.workspace_id=m.workspace_id AND c.message_instance_id=m.message_instance_id AND c.state='complete')",
        params![reader.workspace().as_str(),request.as_str(),now.saturating_sub(crate::managed_text::RETENTION_MS)],
        |r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    let Some((source, occurrence)) = source else {
        return Ok(None);
    };
    let source = LogicalRequestId::parse(source).map_err(|_| ObservationV2Error::Unavailable)?;
    if reader.allowed_runs().is_some()
        && relation::authorized_link(tx, reader, &source, now).is_err()
    {
        return Ok(None);
    }
    Ok(Some(ObservationInputAnchorV1 {
        source_request_id: source,
        message_occurrence_id: occurrence,
    }))
}
