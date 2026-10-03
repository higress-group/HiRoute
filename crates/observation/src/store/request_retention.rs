use super::*;
use hiroute_domain::{
    DeletionDataClass, ObservationQueryError as Error, SessionDeletionSpecV1, SessionId,
};
use rusqlite::params;

impl LocalObservationStore {
    /// Per-request retention, including old requests inside a still-active
    /// session. Archive contribution and visibility commit in one transaction.
    pub fn expire_request_details(&self, now_ms: i64, limit: usize) -> Result<usize, Error> {
        if now_ms < 0 || limit == 0 || limit > 200 {
            return Err(Error::InvalidQuery);
        }
        let cutoff = now_ms.saturating_sub(hiroute_domain::SEVEN_DAYS_MILLIS);
        let mut connection = self.connection.lock();
        let transaction = connection.transaction().map_err(|_| Error::Unavailable)?;
        let quality_changed = expire_plan_quality_details(&transaction, cutoff)?;
        let requests = {
            let mut statement=transaction.prepare("SELECT workspace_id,session_id,request_id FROM logical_requests WHERE started_at_ms<=?1 ORDER BY started_at_ms,request_id LIMIT ?2").map_err(|_|Error::Unavailable)?;
            statement
                .query_map(params![cutoff, limit], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })
                .map_err(|_| Error::Unavailable)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| Error::Corrupt)?
        };
        for (workspace, session, request) in &requests {
            let spec = SessionDeletionSpecV1 {
                workspace_id: WorkspaceId::parse(workspace).map_err(|_| Error::Corrupt)?,
                session_id: SessionId::parse(session).map_err(|_| Error::Corrupt)?,
                data_class: DeletionDataClass::FactsAndContent,
                delete_rollups: false,
            };
            // Opaque archive contribution identity has no original session/run
            // reverse lookup. Exact retries use the same authority-derived key.
            let archive = self.authority.content_blob_digest(
                "retention-archive/v2",
                format!("{workspace}\0{request}").as_bytes(),
            );
            let archive_id =
                SessionId::parse(format!("archive-{archive}")).map_err(|_| Error::Corrupt)?;
            super::value_rollup::archive_request_value(&transaction, &spec, request, archive_id)?;
            crate::valuation::archive_request(&transaction, workspace, session, request)
                .map_err(|_| Error::Unavailable)?;
            transaction.execute("INSERT OR IGNORE INTO observation_request_tombstones_v2(workspace_id,request_id,deleted_ms) VALUES(?1,?2,?3)",params![workspace,request,now_ms]).map_err(|_|Error::Unavailable)?;
            transaction.execute("DELETE FROM content_chunks_v2 WHERE workspace_id=?1 AND content_id IN(SELECT content_id FROM content_instances_v2 WHERE workspace_id=?1 AND request_id=?2)",params![workspace,request]).map_err(|_|Error::Unavailable)?;
            for table in [
                "observation_sensitive_payloads_v2",
                "content_instances_v2",
                "content_message_instances_v2",
                "request_content_refs_v2",
                "conversation_content_streams_v2",
                "conversation_content_events_v2",
                "transcript_roots_v2",
                "observation_run_links",
                "observation_link_events",
                "observation_attempt_models_v2",
                "observation_safe_facts_v2",
                "execution_fact_events",
                "routing_receipts",
                "attempts",
                "value_ledger_entries",
                "logical_requests",
            ] {
                transaction
                    .execute(
                        &format!("DELETE FROM {table} WHERE workspace_id=?1 AND request_id=?2"),
                        params![workspace, request],
                    )
                    .map_err(|_| Error::Unavailable)?;
            }
            transaction.execute("DELETE FROM turns WHERE workspace_id=?1 AND session_id=?2 AND NOT EXISTS(SELECT 1 FROM logical_requests r WHERE r.workspace_id=?1 AND r.turn_id=turns.turn_id)",params![workspace,session]).map_err(|_|Error::Unavailable)?;
            transaction.execute("UPDATE sessions SET content_completeness='expired',facts_completeness='unknown',tombstone_reason='retention_expired',agent_id='',correlation='unproven' WHERE workspace_id=?1 AND session_id=?2 AND NOT EXISTS(SELECT 1 FROM logical_requests WHERE workspace_id=?1 AND session_id=?2)",params![workspace,session]).map_err(|_|Error::Unavailable)?;
        }
        // Removing the only Partial/Unknown request may improve completeness.
        // Rebuild once per affected session, never once per newly ingested fact.
        let sessions: std::collections::BTreeSet<_> = requests
            .iter()
            .map(|(workspace, session, _)| (workspace, session))
            .collect();
        for (workspace, session) in sessions {
            let completeness = crate::receipt::session_facts_completeness(
                &transaction,
                &WorkspaceId::parse(workspace).map_err(|_| Error::Corrupt)?,
                &SessionId::parse(session).map_err(|_| Error::Corrupt)?,
            )
            .map_err(|_| Error::Unavailable)?;
            let value = match completeness {
                hiroute_domain::FactsCompleteness::Complete => "complete",
                hiroute_domain::FactsCompleteness::Partial => "partial",
                hiroute_domain::FactsCompleteness::Unknown => "unknown",
            };
            transaction.execute("UPDATE sessions SET facts_completeness=?3 WHERE workspace_id=?1 AND session_id=?2",
                params![workspace, session, value]).map_err(|_| Error::Unavailable)?;
        }
        if !requests.is_empty() || quality_changed {
            // Index cleanup is part of the same visibility barrier. Physical
            // blob GC is resumable and can follow this committed logical erase.
            transaction.execute_batch("DELETE FROM observation_text_blocks_v2 WHERE NOT EXISTS(SELECT 1 FROM content_instances_v2 c WHERE c.workspace_id=observation_text_blocks_v2.workspace AND c.content_blob_digest=observation_text_blocks_v2.digest AND c.state='complete'); DELETE FROM observation_text_index_v2 WHERE NOT EXISTS(SELECT 1 FROM content_instances_v2 c WHERE c.workspace_id=observation_text_index_v2.workspace AND c.content_blob_digest=observation_text_index_v2.digest AND c.state='complete');").map_err(|_|Error::Unavailable)?;
            crate::query_v2::invalidate_visibility(&transaction).map_err(|_| Error::Unavailable)?;
            increment_store_revision(&transaction).map_err(|_| Error::Unavailable)?;
        }
        transaction.commit().map_err(|_| Error::Unavailable)?;
        Ok(requests.len())
    }
}

fn expire_plan_quality_details(
    transaction: &rusqlite::Transaction<'_>,
    cutoff: i64,
) -> Result<bool, Error> {
    let mut changed = transaction
        .execute(
            "DELETE FROM plan_quality_segments
             WHERE last_at_ms IS NOT NULL AND last_at_ms<=?1",
            [cutoff],
        )
        .map_err(|_| Error::Unavailable)?;
    changed += transaction
        .execute(
            "UPDATE plan_quality_segments SET
                assessment_event_id=NULL,assessment_sequence=NULL,
                assessment_trigger_request_id=NULL,assessed_at_ms=NULL,
                target_from_turn_id=NULL,target_through_turn_id=NULL,
                target_from_ordinal=NULL,target_through_ordinal=NULL,
                score=NULL,assessment_partial=NULL,reason_present=0
             WHERE assessment_trigger_request_id IN (
                SELECT request_id FROM logical_requests
                 WHERE workspace_id=plan_quality_segments.workspace_id
                   AND started_at_ms<=?1)",
            [cutoff],
        )
        .map_err(|_| Error::Unavailable)?;
    changed += transaction
        .execute(
            "UPDATE plan_quality_segments SET first_request_id=NULL,history_partial=1
             WHERE first_request_id IN (
                SELECT request_id FROM logical_requests
                 WHERE workspace_id=plan_quality_segments.workspace_id
                   AND started_at_ms<=?1)",
            [cutoff],
        )
        .map_err(|_| Error::Unavailable)?;
    changed += transaction
        .execute(
            "DELETE FROM plan_quality_segments
             WHERE selected_branch_id IS NULL AND assessment_event_id IS NULL",
            [],
        )
        .map_err(|_| Error::Unavailable)?;
    if changed > 0 {
        transaction
            .execute(
                "UPDATE observation_meta SET value=CAST(value AS INTEGER)+1
                 WHERE key='plan_quality_generation'",
                [],
            )
            .map_err(|_| Error::Unavailable)?;
    }
    Ok(changed > 0)
}

pub(super) fn retired(
    connection: &rusqlite::Connection,
    workspace: &WorkspaceId,
    request: &hiroute_domain::LogicalRequestId,
) -> Result<bool, crate::writer::ObservationStoreError> {
    connection.query_row("SELECT EXISTS(SELECT 1 FROM observation_request_tombstones_v2 WHERE workspace_id=?1 AND request_id=?2)",params![workspace.as_str(),request.as_str()],|row|row.get(0)).map_err(|_|crate::writer::ObservationStoreError::ActivityUnavailable)
}

/// Called only after validated preflight and inside the projection savepoint.
/// Request tombstones and timestamp barriers already excluded retired origins.
pub(super) fn revive_for_new_request(
    connection: &rusqlite::Connection,
    workspace: &WorkspaceId,
    session: &SessionId,
    request: &hiroute_domain::LogicalRequestId,
) -> Result<(), crate::writer::ObservationStoreError> {
    connection.execute("UPDATE sessions SET tombstone_reason=NULL,content_completeness=CASE WHEN content_completeness IN('deleted','expired') THEN 'unknown' ELSE content_completeness END WHERE workspace_id=?1 AND session_id=?2 AND NOT EXISTS(SELECT 1 FROM logical_requests WHERE workspace_id=?1 AND request_id=?3)",params![workspace.as_str(),session.as_str(),request.as_str()]).map_err(|_|crate::writer::ObservationStoreError::ActivityUnavailable)?;
    Ok(())
}
