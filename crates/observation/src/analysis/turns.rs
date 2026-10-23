use super::schema;
use hiroute_domain::{ExecutionFactEnvelopeV1, ExecutionFactV1};
use rusqlite::{Connection, params};

pub(crate) fn project(db: &Connection, e: &ExecutionFactEnvelopeV1) -> rusqlite::Result<()> {
    if !schema::available(db) {
        return Ok(());
    }
    if let ExecutionFactV1::AgentTurnAdmitted(admission) = &e.fact {
        // Identity is available while execution is in flight. Segment/endpoints
        // remain absent until the actual frozen membership supplies them.
        let digest = hiroute_domain::CanonicalDigest::of(admission)
            .map_err(|_| rusqlite::Error::InvalidQuery)?
            .to_string();
        db.execute(
            "INSERT INTO observation_analysis_turns_v1(workspace_id,request_id,agent_turn_id,ordinal,producer_epoch,partial,branch_id,admission_digest) VALUES(?1,?2,?3,?4,?5,0,?6,?7)
             ON CONFLICT(workspace_id,request_id) DO UPDATE SET
               partial=MAX(partial,agent_turn_id!=excluded.agent_turn_id OR ordinal!=excluded.ordinal OR producer_epoch!=excluded.producer_epoch OR (branch_id IS NOT NULL AND branch_id!=excluded.branch_id) OR (admission_digest IS NOT NULL AND admission_digest!=excluded.admission_digest)),
               membership_conflict=MAX(membership_conflict,agent_turn_id!=excluded.agent_turn_id OR ordinal!=excluded.ordinal OR producer_epoch!=excluded.producer_epoch OR (branch_id IS NOT NULL AND branch_id!=excluded.branch_id) OR (admission_digest IS NOT NULL AND admission_digest!=excluded.admission_digest)),
               branch_id=COALESCE(branch_id,excluded.branch_id),
               admission_digest=COALESCE(admission_digest,excluded.admission_digest)",
            params![e.correlation.workspace_id.as_str(), e.correlation.request_id.as_str(),
                admission.agent_turn_id, admission.ordinal, e.producer.stream.producer_epoch.as_str(), admission.branch_id, digest],
        )?;
        return Ok(());
    }
    let ExecutionFactV1::AgentTurnFinished {
        history_membership,
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
    let rows = if let Some(membership) = history_membership {
        let encoded = serde_json::to_string(&membership.request_ids)
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
        let mut stmt = db.prepare("SELECT r.request_id FROM logical_requests r JOIN json_each(?3) member ON member.value=r.request_id WHERE r.workspace_id=?1 AND r.session_id=?2")?;
        stmt.query_map(params![w, session, encoded], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?
    } else {
        let mut stmt=db.prepare("SELECT r.request_id FROM logical_requests r WHERE r.workspace_id=?1 AND r.session_id=?2
      AND (r.request_id IN (?3,?4) OR
        r.started_at_ms > (SELECT started_at_ms FROM logical_requests WHERE workspace_id=?1 AND request_id=?3 AND session_id=?2)
        AND r.started_at_ms < (SELECT started_at_ms FROM logical_requests WHERE workspace_id=?1 AND request_id=?4 AND session_id=?2))")?;
        stmt.query_map(params![w, session, first.as_str(), last.as_str()], |r| {
            r.get::<_, String>(0)
        })?
        .collect::<Result<Vec<_>, _>>()?
    };
    let partial = *history_partial
        || history_membership
            .as_ref()
            .is_some_and(|m| m.clipped || m.capture_partial || rows.len() != m.request_ids.len());
    for request in rows {
        db.execute(
            "INSERT INTO observation_analysis_turns_v1(workspace_id,request_id,agent_turn_id,ordinal,producer_epoch,partial,segment_id,first_request_id,last_request_id,membership_revision,branch_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?11,?12)
          ON CONFLICT(workspace_id,request_id) DO UPDATE SET
            partial=MAX(partial,excluded.partial,agent_turn_id!=excluded.agent_turn_id OR ordinal!=excluded.ordinal OR producer_epoch!=excluded.producer_epoch OR (branch_id IS NOT NULL AND excluded.branch_id IS NOT NULL AND branch_id!=excluded.branch_id) OR (segment_id IS NOT NULL AND segment_id!=excluded.segment_id) OR ((?10=0 OR (membership_revision>0 AND membership_revision=excluded.membership_revision)) AND ((first_request_id IS NOT NULL AND first_request_id!=excluded.first_request_id) OR (last_request_id IS NOT NULL AND last_request_id!=excluded.last_request_id)))),
            membership_conflict=MAX(membership_conflict,agent_turn_id!=excluded.agent_turn_id OR ordinal!=excluded.ordinal OR producer_epoch!=excluded.producer_epoch OR (branch_id IS NOT NULL AND excluded.branch_id IS NOT NULL AND branch_id!=excluded.branch_id) OR (segment_id IS NOT NULL AND segment_id!=excluded.segment_id) OR ((?10=0 OR (membership_revision>0 AND membership_revision=excluded.membership_revision)) AND ((first_request_id IS NOT NULL AND first_request_id!=excluded.first_request_id) OR (last_request_id IS NOT NULL AND last_request_id!=excluded.last_request_id)))),
            segment_id=COALESCE(segment_id,excluded.segment_id),
            first_request_id=CASE WHEN ?10=1 AND excluded.membership_revision>=membership_revision THEN excluded.first_request_id ELSE COALESCE(first_request_id,excluded.first_request_id) END,
            last_request_id=CASE WHEN ?10=1 AND excluded.membership_revision>=membership_revision THEN excluded.last_request_id ELSE COALESCE(last_request_id,excluded.last_request_id) END,
            membership_revision=MAX(membership_revision,excluded.membership_revision),
            branch_id=COALESCE(branch_id,excluded.branch_id),
               admission_digest=COALESCE(admission_digest,excluded.admission_digest)",
            params![
                w,
                request,
                agent_turn_id,
                ordinal,
                e.producer.stream.producer_epoch.as_str(),
                partial,
                segment_id,
                first.as_str(),
                last.as_str(),
                history_membership.is_some(),
                history_membership.as_ref().map_or(0, |m| m.revision),
                history_membership.as_ref().map(|m| m.branch_id.as_str())
            ],
        )?;
    }
    Ok(())
}
