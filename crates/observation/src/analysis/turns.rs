use super::schema;
use hiroute_domain::{ExecutionFactEnvelopeV1, ExecutionFactV1};
use rusqlite::{Connection, params};

pub(crate) fn project(db: &Connection, e: &ExecutionFactEnvelopeV1) -> rusqlite::Result<()> {
    if !schema::available(db) {
        return Ok(());
    }
    let ExecutionFactV1::AgentTurnFinished {
        agent_turn_id,
        segment_id,
        ordinal,
        history_partial,
        first_request_id: Some(first),
        last_request_id: Some(last),
        ..
    } = &e.fact
    else {
        return Ok(());
    };
    let w = e.correlation.workspace_id.as_str();
    let session = e.correlation.conversation_id.as_str();
    let rows = {
        let mut stmt=db.prepare("SELECT r.request_id FROM logical_requests r WHERE r.workspace_id=?1 AND r.session_id=?2
      AND (r.request_id IN (?3,?4) OR
        r.started_at_ms > (SELECT started_at_ms FROM logical_requests WHERE workspace_id=?1 AND request_id=?3 AND session_id=?2)
        AND r.started_at_ms < (SELECT started_at_ms FROM logical_requests WHERE workspace_id=?1 AND request_id=?4 AND session_id=?2))")?;
        stmt.query_map(params![w, session, first.as_str(), last.as_str()], |r| {
            r.get::<_, String>(0)
        })?
        .collect::<Result<Vec<_>, _>>()?
    };
    for request in rows {
        db.execute(
            "INSERT INTO observation_analysis_turns_v1(workspace_id,request_id,agent_turn_id,ordinal,producer_epoch,partial,segment_id,first_request_id,last_request_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)
          ON CONFLICT(workspace_id,request_id) DO UPDATE SET
            partial=MAX(partial,excluded.partial,agent_turn_id!=excluded.agent_turn_id OR ordinal!=excluded.ordinal OR producer_epoch!=excluded.producer_epoch OR (segment_id IS NOT NULL AND segment_id!=excluded.segment_id) OR (first_request_id IS NOT NULL AND first_request_id!=excluded.first_request_id) OR (last_request_id IS NOT NULL AND last_request_id!=excluded.last_request_id)),
            membership_conflict=MAX(membership_conflict,agent_turn_id!=excluded.agent_turn_id OR ordinal!=excluded.ordinal OR producer_epoch!=excluded.producer_epoch OR (segment_id IS NOT NULL AND segment_id!=excluded.segment_id) OR (first_request_id IS NOT NULL AND first_request_id!=excluded.first_request_id) OR (last_request_id IS NOT NULL AND last_request_id!=excluded.last_request_id)),
            segment_id=COALESCE(segment_id,excluded.segment_id),
            first_request_id=COALESCE(first_request_id,excluded.first_request_id),
            last_request_id=COALESCE(last_request_id,excluded.last_request_id)",
            params![
                w,
                request,
                agent_turn_id,
                ordinal,
                e.producer.stream.producer_epoch.as_str(),
                history_partial,
                segment_id,
                first.as_str(),
                last.as_str()
            ],
        )?;
    }
    Ok(())
}
