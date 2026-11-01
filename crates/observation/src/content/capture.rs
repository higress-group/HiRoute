//! Small archive provenance. The serialized writer claims context once; this
//! state neither proves runtime continuity nor changes assessment membership.
use super::lifecycle::ContentProjectionError as Error;
use crate::store::cached_sql;
use hiroute_domain::{
    ConversationContentDirectionV2 as Direction, ConversationContentEnvelopeV1 as Envelope,
    CorrelationProvenance, SessionScopeV1,
};
use rusqlite::{Transaction, params};

pub(super) fn begin(tx: &Transaction<'_>, event: &Envelope) -> Result<(), Error> {
    if event.direction != Direction::RequestInput || event.capture.is_none() {
        return Ok(());
    }
    let scope = event.correlation.session_scope == SessionScopeV1::Conversation
        && event.correlation.correlation_provenance != CorrelationProvenance::Unproven;
    if scope {
        cached_sql::execute(tx,"INSERT OR IGNORE INTO observation_capture_scopes_v6(workspace_id,session_id,seed_request_id,seed_state) VALUES(?1,?2,?3,'started')",
            params![event.correlation.workspace_id.as_str(),event.correlation.conversation_id.as_str(),event.correlation.request_id.as_str()]).map_err(|_|Error::ActivityStorage)?;
    }
    let anchor = event
        .capture
        .as_ref()
        .and_then(|c| c.input_anchor_id.as_ref())
        .map(|id| id.as_str());
    cached_sql::execute(tx,"INSERT INTO observation_input_anchors_v6(workspace_id,request_id,source_message_id) VALUES(?1,?2,?3)",
        params![event.correlation.workspace_id.as_str(),event.correlation.request_id.as_str(),anchor]).map_err(|_|Error::ActivityStorage)?;
    Ok(())
}

pub(super) fn skip_context(
    tx: &Transaction<'_>,
    event: &Envelope,
    retry: bool,
) -> Result<bool, Error> {
    if event.direction != Direction::RequestInput
        || !event.capture.as_ref().is_some_and(|c| c.context_only)
    {
        return Ok(false);
    }
    let owns:bool=cached_sql::query_row(tx,"SELECT EXISTS(SELECT 1 FROM observation_capture_scopes_v6 WHERE workspace_id=?1 AND session_id=?2 AND seed_request_id=?3 AND (?4 OR seed_state='started'))",
        params![event.correlation.workspace_id.as_str(),event.correlation.conversation_id.as_str(),event.correlation.request_id.as_str(),retry],|r|r.get(0)).map_err(|_|Error::ActivityStorage)?;
    Ok(!owns)
}

pub(super) fn terminal(tx: &Transaction<'_>, event: &Envelope) -> Result<(), Error> {
    if event.direction != Direction::RequestInput || event.capture.is_none() {
        return Ok(());
    }
    cached_sql::execute(tx,"UPDATE observation_capture_scopes_v6 SET seed_state=?4 WHERE workspace_id=?1 AND session_id=?2 AND seed_request_id=?3",
        params![event.correlation.workspace_id.as_str(),event.correlation.conversation_id.as_str(),event.correlation.request_id.as_str(),event.phase.as_str()]).map_err(|_|Error::ActivityStorage)?;
    Ok(())
}
